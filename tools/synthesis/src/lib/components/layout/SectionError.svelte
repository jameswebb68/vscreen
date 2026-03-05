<script lang="ts">
	interface Props {
		sectionId: string;
		component: string;
		error: unknown;
		reset: () => void;
	}

	let { sectionId, component, error, reset }: Props = $props();

	let message = $derived(
		error instanceof Error ? error.message : String(error)
	);
</script>

<div class="rounded-lg border border-red-500/30 bg-red-500/10 p-4">
	<div class="flex items-center justify-between">
		<p class="text-sm text-red-400">
			Section "<span class="font-mono">{sectionId}</span>" ({component}) failed to render
		</p>
		<button
			class="rounded bg-red-500/20 px-3 py-1 text-xs text-red-300 transition hover:bg-red-500/30"
			onclick={reset}
		>
			Retry
		</button>
	</div>
	<pre class="mt-2 text-xs text-red-300/60">{message}</pre>
</div>
