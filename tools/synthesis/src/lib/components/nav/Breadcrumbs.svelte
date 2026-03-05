<script lang="ts">
	import type { BreadcrumbItem } from '$lib/types/index.js';

	interface Props {
		items: BreadcrumbItem[];
	}

	let { items }: Props = $props();
</script>

<nav aria-label="Breadcrumb">
	<ol class="flex flex-wrap items-center gap-1 text-sm text-surface-400">
		{#each items as item, i (item.label)}
			{#if i > 0}
				<li class="select-none text-surface-600" aria-hidden="true">/</li>
			{/if}
			<li>
				{#if item.url && i < items.length - 1}
					<a href={item.url} class="rounded px-1 py-0.5 transition hover:text-surface-100">{item.label}</a>
				{:else}
					<span class="px-1 py-0.5 text-surface-200">{item.label}</span>
				{/if}
			</li>
		{/each}
	</ol>
</nav>
