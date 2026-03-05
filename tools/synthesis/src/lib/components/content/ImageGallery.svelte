<script lang="ts">
	import type { ImageItem } from '$lib/types/index.js';

	interface Props {
		items: ImageItem[];
		title?: string;
		columns?: number;
	}

	let { items, title, columns = 3 }: Props = $props();

	let gridClass = $derived(
		columns === 2
			? 'grid-cols-1 md:grid-cols-2'
			: columns === 4
				? 'grid-cols-2 md:grid-cols-4'
				: 'grid-cols-1 md:grid-cols-2 lg:grid-cols-3'
	);
</script>

{#if title}
	<h2 class="mb-4 text-lg font-bold text-surface-100">{title}</h2>
{/if}

<div class="grid {gridClass} gap-3">
	{#each items as item (item.src)}
		<figure class="overflow-hidden rounded-lg bg-surface-800">
			{#if item.url}
				<a href={item.url} target="_blank" rel="noopener noreferrer">
					<img src={item.src} alt={item.alt ?? ''} class="h-48 w-full object-cover" />
				</a>
			{:else}
				<img src={item.src} alt={item.alt ?? ''} class="h-48 w-full object-cover" />
			{/if}
			{#if item.caption}
				<figcaption class="p-2 text-xs text-surface-400">{item.caption}</figcaption>
			{/if}
		</figure>
	{/each}
</div>
