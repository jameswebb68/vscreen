<script lang="ts">
	import type { ArticleItem } from '$lib/types/index.js';
	import SourceBadge from './SourceBadge.svelte';

	interface Props {
		items: ArticleItem[];
		title?: string;
	}

	let { items, title }: Props = $props();
</script>

{#if title}
	<h2 class="mb-4 text-lg font-bold text-surface-100">{title}</h2>
{/if}

<ul class="space-y-3">
	{#each items as item (item.url ?? item.title)}
		<li>
			<a
				href={item.url ?? '#'}
				target="_blank"
				rel="noopener noreferrer"
				class="flex gap-3 rounded-lg bg-surface-800 p-3 transition hover:bg-surface-700"
			>
				{#if item.image}
					<img
						src={item.image}
						alt={item.title}
						class="h-16 w-24 shrink-0 rounded object-cover"
					/>
				{/if}
				<div class="min-w-0 flex-1">
					<h3 class="text-sm font-semibold leading-snug text-surface-100">{item.title}</h3>
					{#if item.description}
						<p class="mt-1 line-clamp-1 text-xs text-surface-400">{item.description}</p>
					{/if}
					<div class="mt-1 flex items-center gap-2">
						{#if item.source}
							<SourceBadge source={item.source} color={item.sourceColor} />
						{/if}
						{#if item.timestamp}
							<time class="text-xs text-surface-500">{item.timestamp}</time>
						{/if}
					</div>
				</div>
			</a>
		</li>
	{/each}
</ul>
