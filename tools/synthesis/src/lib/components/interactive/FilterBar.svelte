<script lang="ts">
	import type { FilterOption } from '$lib/types/index.js';

	interface Props {
		filters: FilterOption[];
		onToggle: (id: string) => void;
	}

	let { filters, onToggle }: Props = $props();
</script>

<div class="flex flex-wrap gap-2">
	{#each filters as filter (filter.id)}
		<button
			class="rounded-full px-3 py-1.5 text-xs font-medium transition
				{filter.active
					? 'bg-accent-500/20 text-accent-400 ring-1 ring-accent-500/40'
					: 'bg-surface-800 text-surface-400 hover:bg-surface-700 hover:text-surface-200'}"
			onclick={() => onToggle(filter.id)}
		>
			{filter.label}
		</button>
	{/each}
</div>
