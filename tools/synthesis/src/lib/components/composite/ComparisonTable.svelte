<script lang="ts">
	import type { ComparisonColumn, ComparisonFeature } from '$lib/types/index.js';

	interface Props {
		columns: ComparisonColumn[];
		features: ComparisonFeature[];
		title?: string;
	}

	let { columns, features, title }: Props = $props();

	function displayValue(val: string | number | boolean | undefined): string {
		if (val === true) return '✓';
		if (val === false) return '✗';
		if (val === undefined) return '—';
		return String(val);
	}

	function valueClass(val: string | number | boolean | undefined): string {
		if (val === true) return 'text-green-400';
		if (val === false) return 'text-red-400';
		return 'text-surface-200';
	}
</script>

{#if title}
	<h2 class="mb-4 text-lg font-bold text-surface-100">{title}</h2>
{/if}

<div class="overflow-x-auto rounded-lg bg-surface-800">
	<table class="w-full text-sm">
		<thead>
			<tr class="border-b border-surface-700">
				<th class="px-4 py-3 text-left text-xs font-medium uppercase text-surface-500">Feature</th>
				{#each columns as col (col.id)}
					<th class="px-4 py-3 text-center text-xs font-medium uppercase {col.highlight ? 'bg-accent-500/10 text-accent-400' : 'text-surface-400'}">
						{col.label}
					</th>
				{/each}
			</tr>
		</thead>
		<tbody>
			{#each features as feature (feature.label)}
				<tr class="border-b border-surface-700/50">
					<td class="px-4 py-3 text-surface-300">{feature.label}</td>
					{#each columns as col (col.id)}
						<td class="px-4 py-3 text-center {col.highlight ? 'bg-accent-500/5' : ''} {valueClass(feature.values[col.id])}">
							{displayValue(feature.values[col.id])}
						</td>
					{/each}
				</tr>
			{/each}
		</tbody>
	</table>
</div>
