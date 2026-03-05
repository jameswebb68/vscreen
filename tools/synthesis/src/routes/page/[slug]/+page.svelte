<script lang="ts">
	import type { SynthesisPage } from '$lib/types/index.js';
	import PageShell from '$lib/components/layout/PageShell.svelte';
	import ComponentRenderer from '$lib/components/layout/ComponentRenderer.svelte';
	import SectionError from '$lib/components/layout/SectionError.svelte';
	import TabLayout from '$lib/components/layout/TabLayout.svelte';
	import SplitLayout from '$lib/components/layout/SplitLayout.svelte';

	let { data } = $props();

	let page: SynthesisPage = $derived(data.page);
	let pageId = $derived(page.id);
	let _connected = $state(false);

	let layoutClass = $derived(
		page.layout === 'grid'
			? 'space-y-8'
			: page.layout === 'list'
				? 'space-y-4'
				: 'space-y-8'
	);

	$effect(() => {
		const evtSource = new EventSource('/api/events');

		evtSource.onopen = () => {
			_connected = true;
			if (!pageId) return;
			fetch(`/api/pages/${pageId}`)
				.then((res) => (res.ok ? res.json() : Promise.resolve(null)))
				.then((json) => {
					if (json) data.page = json as SynthesisPage;
				})
				.catch(() => {});
		};

		evtSource.onmessage = async (event) => {
			const msg = JSON.parse(event.data as string) as { type: string; pageId?: string };
			if (msg.pageId === pageId && (msg.type === 'updated' || msg.type === 'push')) {
				const res = await fetch(`/api/pages/${pageId}`);
				if (res.ok) {
					data.page = (await res.json()) as SynthesisPage;
				}
			}
		};

		evtSource.onerror = () => {
			_connected = false;
		};

		return () => {
			evtSource.close();
		};
	});

	let leftSection = $derived(page.sections[0]);
	let rightSection = $derived(page.sections[1]);
	let extraSections = $derived(page.sections.slice(2));
</script>

<svelte:head>
	<title>{page.title} — Synthesis</title>
</svelte:head>

<PageShell title={page.title} subtitle={page.subtitle} theme={page.theme}>
	{#if page.layout === 'tabs' && page.sections.length > 1}
		<TabLayout tabs={page.sections.map((s) => ({ id: s.id, label: s.title ?? s.id }))}>
			{#snippet children(activeTab)}
				{#each page.sections as section (section.id)}
					{#if section.id === activeTab}
						<svelte:boundary onerror={(e) => console.error(`Section ${section.id} (${section.component}) failed:`, e)}>
							<ComponentRenderer {section} />
							{#snippet failed(error, reset)}
								<SectionError sectionId={section.id} component={section.component} {error} {reset} />
							{/snippet}
						</svelte:boundary>
					{/if}
				{/each}
			{/snippet}
		</TabLayout>
	{:else if page.layout === 'split' && page.sections.length >= 2 && leftSection && rightSection}
		<SplitLayout>
			{#snippet left()}
				<svelte:boundary onerror={(e) => console.error(`Section ${leftSection.id} (${leftSection.component}) failed:`, e)}>
					<ComponentRenderer section={leftSection} />
					{#snippet failed(error, reset)}
						<SectionError sectionId={leftSection.id} component={leftSection.component} {error} {reset} />
					{/snippet}
				</svelte:boundary>
			{/snippet}
			{#snippet right()}
				<svelte:boundary onerror={(e) => console.error(`Section ${rightSection.id} (${rightSection.component}) failed:`, e)}>
					<ComponentRenderer section={rightSection} />
					{#snippet failed(error, reset)}
						<SectionError sectionId={rightSection.id} component={rightSection.component} {error} {reset} />
					{/snippet}
				</svelte:boundary>
			{/snippet}
		</SplitLayout>
		{#if extraSections.length > 0}
			<div class="mt-8 space-y-8">
				{#each extraSections as section (section.id)}
					<svelte:boundary onerror={(e) => console.error(`Section ${section.id} (${section.component}) failed:`, e)}>
						<ComponentRenderer {section} />
						{#snippet failed(error, reset)}
							<SectionError sectionId={section.id} component={section.component} {error} {reset} />
						{/snippet}
					</svelte:boundary>
				{/each}
			</div>
		{/if}
	{:else}
		<div class={layoutClass}>
			{#each page.sections as section (section.id)}
				<svelte:boundary onerror={(e) => console.error(`Section ${section.id} (${section.component}) failed:`, e)}>
					<ComponentRenderer {section} />
					{#snippet failed(error, reset)}
						<SectionError sectionId={section.id} component={section.component} {error} {reset} />
					{/snippet}
				</svelte:boundary>
			{/each}
		</div>
	{/if}
</PageShell>