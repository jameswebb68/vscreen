<script lang="ts">
	import type { FilterOption } from '$lib/types/index.js';
	import FilterBar from '../../interactive/FilterBar.svelte';

	interface Props {
		filters: FilterOption[];
	}

	let { filters }: Props = $props();
</script>

<FilterBar
	{filters}
	onToggle={(id) => {
		const f = filters.find((x) => x.id === id);
		if (f) f.active = !f.active;
	}}
/>
