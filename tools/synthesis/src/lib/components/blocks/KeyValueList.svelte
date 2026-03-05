<script lang="ts">
	import type { KeyValuePair } from '$lib/types/index.js';

	interface Props {
		items: KeyValuePair[];
		title?: string;
	}

	let { items, title }: Props = $props();
</script>

{#if title}
	<h2 class="mb-4 text-lg font-bold text-surface-100">{title}</h2>
{/if}

<dl class="divide-y divide-surface-700 rounded-lg bg-surface-800">
	{#each items as item (item.key)}
		<div class="flex items-center justify-between px-4 py-3">
			<dt class="text-sm text-surface-400">{item.key}</dt>
			<dd class="text-sm font-medium text-surface-200">
				{#if item.url}
					<a href={item.url} target="_blank" rel="noopener noreferrer" class="text-accent-400 hover:underline">{item.value}</a>
				{:else}
					{item.value}
				{/if}
			</dd>
		</div>
	{/each}
</dl>
