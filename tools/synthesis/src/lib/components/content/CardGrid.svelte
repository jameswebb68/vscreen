<script lang="ts">
	import type { Snippet } from 'svelte';
	import type { ArticleItem } from '$lib/types/index.js';
	import ArticleCard from './ArticleCard.svelte';

	interface Props {
		items: ArticleItem[];
		title?: string;
		columns?: number;
		header?: Snippet;
	}

	let { items, title, columns = 4, header }: Props = $props();

	let gridClass = $derived(
		columns === 2
			? 'grid-cols-1 md:grid-cols-2'
			: columns === 3
				? 'grid-cols-1 md:grid-cols-2 lg:grid-cols-3'
				: 'grid-cols-1 md:grid-cols-2 lg:grid-cols-4'
	);
</script>

{#if header}
	{@render header()}
{:else if title}
	<h2 class="mb-4 text-lg font-bold text-surface-100">{title}</h2>
{/if}

<div class="grid {gridClass} gap-4">
	{#each items as item (item.url ?? item.title)}
		<ArticleCard {item} />
	{/each}
</div>
