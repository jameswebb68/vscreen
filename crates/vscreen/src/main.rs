mod dev;
mod mcp_proxy;

use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use vscreen_core::config::AppConfig;
use vscreen_core::instance::{InstanceConfig, InstanceId, InstanceState};
use vscreen_server::{build_router, AppState, InstanceSupervisor};

/// vscreen — Virtual Screen Media Bridge
#[derive(Debug, Parser)]
#[command(name = "vscreen", version, about)]
struct Cli {
    /// Path to the TOML configuration file
    #[arg(short, long, env = "VSCREEN_CONFIG")]
    config: Option<PathBuf>,

    /// Listen address (overrides config file)
    #[arg(short, long, env = "VSCREEN_LISTEN")]
    listen: Option<String>,

    /// Log level (overrides config file)
    #[arg(long, env = "VSCREEN_LOG_LEVEL", default_value = "info")]
    log_level: String,

    /// Output logs as JSON
    #[arg(long, env = "VSCREEN_LOG_JSON")]
    log_json: bool,

    /// Start in dev mode: spawn Xvfb, PulseAudio sink, and Chromium
    #[arg(long)]
    dev: bool,

    /// URL to navigate to in dev mode
    #[arg(long)]
    dev_url: Option<String>,

    /// X11 display number for dev mode (default: 99)
    #[arg(long, default_value = "99")]
    dev_display: u32,

    /// CDP debugging port for dev mode Chromium (default: 9222)
    #[arg(long, default_value = "9222")]
    dev_cdp_port: u16,

    /// Start MCP server on stdin/stdout (for subprocess spawning by MCP clients)
    #[arg(long)]
    mcp_stdio: bool,

    /// Start MCP server on HTTP/SSE at the given address (e.g. 0.0.0.0:8451)
    #[arg(long)]
    mcp_sse: Option<String>,

    /// Lightweight stdio proxy: bridge stdin/stdout to an existing SSE MCP server.
    /// Does NOT start dev mode or any pipelines — just forwards MCP messages.
    /// Example: --mcp-stdio-proxy http://localhost:8451/mcp
    #[arg(long)]
    mcp_stdio_proxy: Option<String>,

    /// Bypass all lock checks in MCP (single-agent mode). Use when only one agent controls instances.
    #[arg(long)]
    mcp_single_agent: bool,

    /// Start the RTSP audio media server on this port (default: 8554).
    /// Enables external RTSP pull-based audio streaming for all instances.
    #[arg(long, default_value = "8554")]
    rtsp_port: u16,

    /// Disable the RTSP audio server entirely.
    #[arg(long)]
    no_rtsp: bool,

    /// Video codec for encoding: h264 (default, best compatibility) or vp9.
    #[arg(long, default_value = "h264")]
    video_codec: String,

    /// Vision LLM URL for screenshot analysis (Ollama or OpenAI-compatible).
    /// Example: VSCREEN_VISION_URL=http://spark.ms.sswt.org:11434
    #[arg(long, env = "VSCREEN_VISION_URL")]
    vision_url: Option<String>,

    /// Vision model name for screenshot analysis.
    /// Example: VSCREEN_VISION_MODEL=qwen3-vl:8b
    #[arg(long, env = "VSCREEN_VISION_MODEL", default_value = "qwen3-vl:8b")]
    vision_model: String,
}

fn main() -> anyhow::Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("failed to install rustls crypto provider");

    let cli = Cli::parse();

    let mut config = if let Some(path) = &cli.config {
        AppConfig::from_file(path).map_err(|e| anyhow::anyhow!("config load failed: {e}"))?
    } else {
        AppConfig::default()
    };

    if let Some(listen) = &cli.listen {
        config.server.listen = listen.clone();
    }
    config.logging.level = cli.log_level.clone();
    config.logging.json = cli.log_json;

    config
        .validate()
        .map_err(|errors| {
            for e in &errors {
                eprintln!("config error: {e}");
            }
            anyhow::anyhow!("{} config validation error(s)", errors.len())
        })?;

    init_tracing(&config.logging.level, config.logging.json);

    // If --mcp-stdio-proxy is specified, run the lightweight proxy and exit.
    // This mode does NOT start dev mode, pipelines, or the HTTP server.
    if let Some(ref proxy_url) = cli.mcp_stdio_proxy {
        let url = proxy_url.clone();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|e| anyhow::anyhow!("failed to build tokio runtime: {e}"))?;

        return runtime.block_on(async {
            mcp_proxy::run_stdio_proxy(&url).await
                .map_err(|e| anyhow::anyhow!("MCP stdio proxy error: {e}"))
        });
    }

    info!(
        listen = %config.server.listen,
        max_instances = config.limits.max_instances,
        dev = cli.dev,
        "starting vscreen"
    );

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| anyhow::anyhow!("failed to build tokio runtime: {e}"))?;

    let video_codec: vscreen_core::VideoCodec = cli.video_codec.parse()
        .unwrap_or_else(|_| {
            eprintln!("unknown --video-codec '{}', using h264", cli.video_codec);
            vscreen_core::VideoCodec::H264
        });

    runtime.block_on(run(
        config,
        cli.dev,
        cli.dev_url,
        cli.dev_display,
        cli.dev_cdp_port,
        cli.mcp_stdio,
        cli.mcp_sse,
        cli.mcp_single_agent,
        cli.rtsp_port,
        cli.no_rtsp,
        video_codec,
        cli.vision_url,
        cli.vision_model,
    ))
}

async fn run(
    config: AppConfig,
    dev_mode: bool,
    dev_url: Option<String>,
    dev_display: u32,
    dev_cdp_port: u16,
    mcp_stdio: bool,
    mcp_sse: Option<String>,
    mcp_single_agent: bool,
    rtsp_port: u16,
    no_rtsp: bool,
    video_codec: vscreen_core::VideoCodec,
    vision_url: Option<String>,
    vision_model: String,
) -> anyhow::Result<()> {
    let cancel = CancellationToken::new();
    let mut state = AppState::new(config.clone(), cancel.clone());
    state.single_agent_mode = mcp_single_agent;
    if !no_rtsp {
        state.rtsp_port = rtsp_port;
    }

    if let Some(url) = vision_url {
        let vision_config = vscreen_server::vision::VisionConfig {
            url,
            model: vision_model,
            api_format: None,
        };
        state.vision_client = Some(vscreen_server::vision::VisionClient::new(vision_config));
    }

    let mut _dev_env = None;

    if dev_mode {
        info!("starting dev environment...");

        let env = dev::DevEnvironment::start(dev_display, dev_cdp_port)
            .await
            .map_err(|e| anyhow::anyhow!("dev environment start failed: {e}"))?;

        let cdp_endpoint = env.cdp_endpoint()
            .ok_or_else(|| anyhow::anyhow!("CDP endpoint not available"))?
            .to_owned();
        let monitor_source = env.monitor_source();

        info!(cdp = %cdp_endpoint, source = %monitor_source, "dev environment ready");

        let mut video_cfg = vscreen_core::config::VideoConfig::default();
        video_cfg.codec = video_codec;

        let instance_config = InstanceConfig {
            instance_id: InstanceId::from("dev"),
            cdp_endpoint,
            pulse_source: monitor_source,
            display: Some(format!(":{dev_display}")),
            video: video_cfg,
            audio: vscreen_core::config::AudioConfig::default(),
            rtp_output: Some(vscreen_core::config::RtpOutputConfig {
                address: "127.0.0.1".into(),
                port: 5004,
                multicast: false,
            }),
        };

        // Create the instance in the registry
        let _entry = state.registry.create(instance_config.clone(), config.limits.max_instances)
            .map_err(|e| anyhow::anyhow!("failed to create dev instance: {e}"))?;

        // Start the supervisor
        match InstanceSupervisor::start(instance_config).await {
            Ok(sup) => {
                let _ = state.registry.get(&InstanceId::from("dev"))
                    .map(|entry| entry.state_tx.send(InstanceState::Running));
                state.set_supervisor("dev", sup);
                info!("dev instance pipeline started");
            }
            Err(e) => {
                error!(?e, "failed to start dev instance supervisor");
            }
        }

        // Navigate to dev URL if specified
        if let Some(url) = &dev_url {
            if let Some(sup) = state.get_supervisor(&InstanceId::from("dev")) {
                if let Err(e) = sup.navigate(url).await {
                    error!(?e, url, "failed to navigate dev instance");
                } else {
                    info!(url, "dev instance navigated");
                }
            }
        }

        _dev_env = Some(env);
    }

    vscreen_server::metrics::init_metrics();

    // --- MCP Server ---
    if let Some(ref sse_addr) = mcp_sse {
        let mcp_state = state.clone();
        let addr = sse_addr.clone();
        tokio::spawn(async move {
            if let Err(e) = vscreen_server::mcp::run_mcp_sse(mcp_state, &addr).await {
                error!(?e, "MCP SSE server error");
            }
        });
    }

    if mcp_stdio {
        let mcp_state = state.clone();
        tokio::spawn(async move {
            if let Err(e) = vscreen_server::mcp::run_mcp_stdio(mcp_state).await {
                error!(?e, "MCP stdio server error");
            }
        });
    }

    // --- RTSP Audio Server ---
    if !no_rtsp {
        let server_ip = std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED);
        let rtsp_state = state.clone();
        let rtsp_cancel = cancel.clone();

        let rtsp_server = vscreen_rtsp::RtspServer::new(
            rtsp_port,
            server_ip,
            std::sync::Arc::new(rtsp_state.clone()),
            rtsp_cancel,
        );

        // Share the session manager with state for REST/MCP access
        state.rtsp_session_manager = Some(rtsp_server.session_manager().clone());

        tokio::spawn(async move {
            if let Err(e) = rtsp_server.run().await {
                error!(?e, "RTSP server error");
            }
        });

        info!(port = rtsp_port, "RTSP audio server started");
    }

    let supervisors_handle = state.supervisors.clone();
    let router = build_router(state);

    let cancel_for_signal = cancel.clone();
    tokio::spawn(async move {
        shutdown_signal().await;
        info!("shutdown signal received");
        cancel_for_signal.cancel();
    });

    #[cfg(feature = "tls")]
    if let Some(ref tls_config) = config.server.tls {
        info!(
            addr = %config.server.listen,
            cert = %tls_config.cert_path,
            "server listening (TLS)"
        );
        let rustls_config = load_rustls_config(tls_config)
            .map_err(|e| anyhow::anyhow!("TLS config error: {e}"))?;
        axum_server::bind_rustls(
            config.server.listen.parse().map_err(|e| anyhow::anyhow!("bad listen addr: {e}"))?,
            rustls_config,
        )
        .serve(router.into_make_service())
        .await
        .map_err(|e| anyhow::anyhow!("TLS server error: {e}"))?;
    } else {
        let listener = TcpListener::bind(&config.server.listen)
            .await
            .map_err(|e| anyhow::anyhow!("failed to bind {}: {e}", config.server.listen))?;
        info!(addr = %config.server.listen, "server listening");
        axum::serve(listener, router)
            .with_graceful_shutdown(cancel.cancelled_owned())
            .await
            .map_err(|e| anyhow::anyhow!("server error: {e}"))?;
    }

    #[cfg(not(feature = "tls"))]
    {
        if config.server.tls.is_some() {
            return Err(anyhow::anyhow!(
                "TLS configured but the 'tls' feature is not enabled. \
                 Rebuild with: cargo build --features tls"
            ));
        }
        let listener = TcpListener::bind(&config.server.listen)
            .await
            .map_err(|e| anyhow::anyhow!("failed to bind {}: {e}", config.server.listen))?;
        info!(addr = %config.server.listen, "server listening");
        axum::serve(listener, router)
            .with_graceful_shutdown(cancel.cancelled_owned())
            .await
            .map_err(|e| anyhow::anyhow!("server error: {e}"))?;
    }

    info!("server stopped, cleaning up...");

    for entry in supervisors_handle.iter() {
        entry.value().stop().await;
    }
    supervisors_handle.clear();

    drop(_dev_env);

    tokio::time::sleep(Duration::from_millis(100)).await;

    info!("shutdown complete");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}

#[cfg(feature = "tls")]
fn load_rustls_config(
    tls: &vscreen_core::config::TlsConfig,
) -> Result<axum_server::tls_rustls::RustlsConfig, String> {
    use axum_server::tls_rustls::RustlsConfig;
    let config = tokio::runtime::Handle::current()
        .block_on(RustlsConfig::from_pem_file(&tls.cert_path, &tls.key_path))
        .map_err(|e| format!("load TLS certs: {e}"))?;
    Ok(config)
}

fn init_tracing(level: &str, json: bool) {
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::prelude::*;

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("vscreen={level},tower_http=info")));

    if json {
        let subscriber = tracing_subscriber::registry()
            .with(filter)
            .with(tracing_subscriber::fmt::layer().json());
        tracing::subscriber::set_global_default(subscriber)
            .expect("failed to set tracing subscriber");
    } else {
        let subscriber = tracing_subscriber::registry()
            .with(filter)
            .with(tracing_subscriber::fmt::layer());
        tracing::subscriber::set_global_default(subscriber)
            .expect("failed to set tracing subscriber");
    }
}
