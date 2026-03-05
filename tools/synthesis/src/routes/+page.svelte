<script lang="ts">
	let { data } = $props();
</script>

<svelte:head>
	<title>Synthesis — vscreen</title>
</svelte:head>

<div class="dark min-h-screen bg-surface-950 p-8">
	<div class="mx-auto max-w-4xl">
		<h1 class="mb-2 text-2xl font-bold text-surface-100">Synthesis</h1>
		<p class="mb-8 text-sm text-surface-400">AI-driven frontend workspace. Pages appear here when created via the API.</p>

		{#if data.pages.length === 0}
			<div class="rounded-lg border border-dashed border-surface-700 p-12 text-center">
				<p class="text-surface-500">No synthesis pages yet.</p>
				<p class="mt-1 text-xs text-surface-600">
					Use the <code class="rounded bg-surface-800 px-1.5 py-0.5">vscreen_synthesis_create</code> MCP tool to build one.
				</p>
			</div>
		{:else}
			<div class="grid gap-4 md:grid-cols-2">
				{#each data.pages as page (page.id)}
					<a
						href="/page/{page.id}"
						class="block rounded-lg bg-surface-800 p-4 transition hover:bg-surface-700"
					>
						<h2 class="font-semibold text-surface-100">{page.title}</h2>
						{#if page.subtitle}
							<p class="mt-1 text-sm text-surface-400">{page.subtitle}</p>
						{/if}
						<div class="mt-3 flex items-center gap-3 text-xs text-surface-500">
							<span>{page.sections.length} section{page.sections.length !== 1 ? 's' : ''}</span>
							<span>{page.layout}</span>
							<span>{page.theme}</span>
						</div>
					</a>
				{/each}
			</div>
		{/if}
	</div>
</div>
