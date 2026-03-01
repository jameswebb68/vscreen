mod common;

use vscreen_core::instance::InstanceId;

use crate::common::fixtures::test_instance_config;
use crate::common::harness::TestServer;

#[tokio::test]
async fn create_instance_via_registry() {
    let server = TestServer::start().await;

    let config = test_instance_config("test-1");
    let max = server.state.config.limits.max_instances;
    let result = server.state.registry.create(config, max);
    assert!(result.is_ok());

    let ids = server.state.registry.list_ids();
    assert_eq!(ids.len(), 1);

    server.stop().await;
}

#[tokio::test]
async fn delete_instance_via_registry() {
    let server = TestServer::start().await;

    let config = test_instance_config("test-del");
    let max = server.state.config.limits.max_instances;
    server.state.registry.create(config, max).expect("create");

    let result = server.state.registry.remove(&InstanceId::from("test-del"));
    assert!(result.is_ok());
    assert!(server.state.registry.is_empty());

    server.stop().await;
}

#[tokio::test]
async fn create_duplicate_rejected() {
    let server = TestServer::start().await;

    let config = test_instance_config("dup");
    let max = server.state.config.limits.max_instances;
    server.state.registry.create(config, max).expect("first");

    let config2 = test_instance_config("dup");
    let result = server.state.registry.create(config2, max);
    assert!(result.is_err());

    server.stop().await;
}

#[tokio::test]
async fn instance_limit_enforced() {
    let mut app_config = vscreen_core::config::AppConfig::default();
    app_config.limits.max_instances = 2;

    let server = TestServer::start_with_config(app_config).await;
    let max = server.state.config.limits.max_instances;

    server
        .state
        .registry
        .create(test_instance_config("a"), max)
        .expect("a");
    server
        .state
        .registry
        .create(test_instance_config("b"), max)
        .expect("b");

    let result = server.state.registry.create(test_instance_config("c"), max);
    assert!(result.is_err());

    server.stop().await;
}

#[tokio::test]
async fn list_instances() {
    let server = TestServer::start().await;
    let max = server.state.config.limits.max_instances;

    server
        .state
        .registry
        .create(test_instance_config("x"), max)
        .expect("x");
    server
        .state
        .registry
        .create(test_instance_config("y"), max)
        .expect("y");

    let ids = server.state.registry.list_ids();
    assert_eq!(ids.len(), 2);

    server.stop().await;
}

#[tokio::test]
async fn health_check() {
    let server = TestServer::start().await;
    let max = server.state.config.limits.max_instances;

    server
        .state
        .registry
        .create(test_instance_config("health-test"), max)
        .expect("create");

    let entry = server
        .state
        .registry
        .get(&InstanceId::from("health-test"))
        .expect("get");

    let state = entry.state_rx.borrow().clone();
    assert_eq!(state, vscreen_core::instance::InstanceState::Created);

    server.stop().await;
}
