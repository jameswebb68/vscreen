<script lang="ts">
	import type { AccordionItem } from '$lib/types/index.js';

	interface Props {
		items: AccordionItem[];
		title?: string;
	}

	let { items, title }: Props = $props();

	let openIndex = $state<number | null>(null);

	function toggle(i: number) {
		openIndex = openIndex === i ? null : i;
	}
</script>

{#if title}
	<h2 class="mb-4 text-lg font-bold text-surface-100">{title}</h2>
{/if}

<div class="divide-y divide-surface-700 rounded-lg bg-surface-800">
	{#each items as item, i (item.title)}
		<div>
			<button
				class="flex w-full items-center justify-between px-4 py-3 text-left text-sm font-medium text-surface-200 transition hover:text-surface-100"
				onclick={() => toggle(i)}
			>
				<span>{item.title}</span>
				<span class="ml-2 text-surface-500 transition-transform {openIndex === i ? 'rotate-180' : ''}">▾</span>
			</button>
			{#if openIndex === i}
				<div class="px-4 pb-4 text-sm text-surface-400">
					{item.content}
				</div>
			{/if}
		</div>
	{/each}
</div>
