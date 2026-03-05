<script lang="ts">
	import type { StatItem } from '$lib/types/index.js';

	interface Props {
		items: StatItem[];
		title?: string;
	}

	let { items, title }: Props = $props();
</script>

{#if title}
	<h2 class="mb-4 text-lg font-bold text-surface-100">{title}</h2>
{/if}

<div class="grid grid-cols-2 gap-3 md:grid-cols-4">
	{#each items as stat (stat.label)}
		<div class="rounded-lg bg-surface-800 p-4 text-center">
			<p class="text-2xl font-bold text-surface-100">
				{stat.value}{#if stat.unit}<span class="ml-1 text-sm text-surface-400">{stat.unit}</span>{/if}
			</p>
			<p class="mt-1 text-xs text-surface-400">{stat.label}</p>
			{#if stat.trend}
				<span
					class="mt-1 inline-block text-xs font-semibold {stat.trend === 'up'
						? 'text-green-400'
						: stat.trend === 'down'
							? 'text-red-400'
							: 'text-surface-500'}"
				>
					{stat.trend === 'up' ? '↑' : stat.trend === 'down' ? '↓' : '—'}
				</span>
			{/if}
		</div>
	{/each}
</div>
