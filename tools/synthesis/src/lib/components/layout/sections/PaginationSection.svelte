<script lang="ts">
	import Pagination from '../../nav/Pagination.svelte';

	interface Props {
		totalPages: number;
	}

	let { totalPages }: Props = $props();

	let currentPage = $state(1);
</script>

<Pagination
	{currentPage}
	{totalPages}
	onPageChange={(p) => (currentPage = p)}
/>
