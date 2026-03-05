<script lang="ts">
	interface Props {
		currentPage: number;
		totalPages: number;
		onPageChange: (page: number) => void;
	}

	let { currentPage, totalPages, onPageChange }: Props = $props();

	let visiblePages = $derived.by(() => {
		const pages: number[] = [];
		const start = Math.max(1, currentPage - 2);
		const end = Math.min(totalPages, currentPage + 2);
		for (let i = start; i <= end; i++) pages.push(i);
		return pages;
	});
</script>

<nav class="flex items-center gap-1" aria-label="Pagination">
	<button
		class="rounded px-3 py-1.5 text-sm text-surface-400 transition hover:bg-surface-700 hover:text-surface-200 disabled:opacity-40"
		disabled={currentPage <= 1}
		onclick={() => onPageChange(currentPage - 1)}
	>Prev</button>

	{#if visiblePages[0] !== undefined && visiblePages[0] > 1}
		<button class="rounded px-3 py-1.5 text-sm text-surface-400 hover:bg-surface-700" onclick={() => onPageChange(1)}>1</button>
		{#if visiblePages[0] > 2}
			<span class="px-1 text-surface-600">…</span>
		{/if}
	{/if}

	{#each visiblePages as page (page)}
		<button
			class="rounded px-3 py-1.5 text-sm transition {page === currentPage
				? 'bg-accent-500/20 font-medium text-accent-400'
				: 'text-surface-400 hover:bg-surface-700 hover:text-surface-200'}"
			onclick={() => onPageChange(page)}
		>{page}</button>
	{/each}

	{#if visiblePages[visiblePages.length - 1] !== undefined && (visiblePages[visiblePages.length - 1] ?? 0) < totalPages}
		{#if (visiblePages[visiblePages.length - 1] ?? 0) < totalPages - 1}
			<span class="px-1 text-surface-600">…</span>
		{/if}
		<button class="rounded px-3 py-1.5 text-sm text-surface-400 hover:bg-surface-700" onclick={() => onPageChange(totalPages)}>{totalPages}</button>
	{/if}

	<button
		class="rounded px-3 py-1.5 text-sm text-surface-400 transition hover:bg-surface-700 hover:text-surface-200 disabled:opacity-40"
		disabled={currentPage >= totalPages}
		onclick={() => onPageChange(currentPage + 1)}
	>Next</button>
</nav>
