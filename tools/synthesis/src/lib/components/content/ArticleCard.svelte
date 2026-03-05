<script lang="ts">
	import type { ArticleItem } from '$lib/types/index.js';
	import SourceBadge from './SourceBadge.svelte';

	interface Props {
		item: ArticleItem;
	}

	let { item }: Props = $props();
</script>

<a
	href={item.url ?? '#'}
	target="_blank"
	rel="noopener noreferrer"
	class="block overflow-hidden rounded-lg bg-surface-800 transition hover:bg-surface-700"
>
	{#if item.image}
		<img src={item.image} alt={item.title} class="h-36 w-full object-cover" />
	{:else}
		<div class="flex h-36 w-full items-center justify-center bg-surface-700 text-sm text-surface-400">
			No image
		</div>
	{/if}
	<div class="p-3">
		{#if item.source}
			<div class="mb-2">
				<SourceBadge source={item.source} color={item.sourceColor} />
			</div>
		{/if}
		<h3 class="text-sm font-semibold leading-snug text-surface-100">{item.title}</h3>
		{#if item.description}
			<p class="mt-1 line-clamp-2 text-xs text-surface-400">{item.description}</p>
		{/if}
		{#if item.timestamp}
			<time class="mt-2 block text-xs text-surface-500">{item.timestamp}</time>
		{/if}
	</div>
</a>
