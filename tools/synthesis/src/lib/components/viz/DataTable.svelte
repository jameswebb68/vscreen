<script lang="ts">
	import type { TableColumn, TableRow } from '$lib/types/index.js';

	interface Props {
		columns: TableColumn[];
		rows: TableRow[];
		title?: string;
		pageSize?: number;
	}

	let { columns, rows, title, pageSize = 20 }: Props = $props();

	let sortKey = $state('');
	let sortAsc = $state(true);
	let currentPage = $state(0);

	let sorted = $derived.by(() => {
		if (!sortKey) return rows;
		const col = columns.find((c) => c.key === sortKey);
		if (!col?.sortable) return rows;
		return [...rows].sort((a, b) => {
			const va = a[sortKey] ?? '';
			const vb = b[sortKey] ?? '';
			const cmp = String(va).localeCompare(String(vb), undefined, { numeric: true });
			return sortAsc ? cmp : -cmp;
		});
	});

	let totalPages = $derived(Math.max(1, Math.ceil(sorted.length / pageSize)));
	let paged = $derived(sorted.slice(currentPage * pageSize, (currentPage + 1) * pageSize));

	function toggleSort(key: string) {
		if (sortKey === key) {
			sortAsc = !sortAsc;
		} else {
			sortKey = key;
			sortAsc = true;
		}
		currentPage = 0;
	}
</script>

{#if title}
	<h2 class="mb-4 text-lg font-bold text-surface-100">{title}</h2>
{/if}

<div class="overflow-x-auto rounded-lg bg-surface-800">
	<table class="w-full text-left text-sm">
		<thead class="border-b border-surface-700 text-xs uppercase text-surface-400">
			<tr>
				{#each columns as col (col.key)}
					<th
						class="px-4 py-3 {col.sortable ? 'cursor-pointer select-none hover:text-surface-200' : ''}"
						style={col.width ? `width: ${col.width}` : ''}
						onclick={() => col.sortable && toggleSort(col.key)}
					>
						{col.label}
						{#if col.sortable && sortKey === col.key}
							<span class="ml-1">{sortAsc ? '↑' : '↓'}</span>
						{/if}
					</th>
				{/each}
			</tr>
		</thead>
		<tbody>
			{#each paged as row, i (i)}
				<tr class="border-b border-surface-700/50 hover:bg-surface-700/30">
					{#each columns as col (col.key)}
						<td class="px-4 py-3 text-surface-200">{row[col.key] ?? ''}</td>
					{/each}
				</tr>
			{/each}
		</tbody>
	</table>

	{#if totalPages > 1}
		<div class="flex items-center justify-between border-t border-surface-700 px-4 py-2 text-xs text-surface-400">
			<span>{sorted.length} rows</span>
			<div class="flex gap-1">
				<button class="rounded px-2 py-1 hover:bg-surface-700" disabled={currentPage === 0} onclick={() => (currentPage = Math.max(0, currentPage - 1))}>Prev</button>
				<span class="px-2 py-1">{currentPage + 1} / {totalPages}</span>
				<button class="rounded px-2 py-1 hover:bg-surface-700" disabled={currentPage >= totalPages - 1} onclick={() => (currentPage = Math.min(totalPages - 1, currentPage + 1))}>Next</button>
			</div>
		</div>
	{/if}
</div>
