<script lang="ts">
	import ProgressBar from '../../viz/ProgressBar.svelte';

	interface Props {
		items: { value: number; max?: number; label?: string; color?: string }[];
		title?: string;
	}

	let { items, title }: Props = $props();
</script>

<div class="space-y-3">
	{#if title}
		<h2 class="text-lg font-bold text-surface-100">{title}</h2>
	{/if}
	{#each items as item (item.label ?? item.value)}
		<ProgressBar value={item.value} max={item.max} label={item.label} color={item.color} />
	{/each}
</div>
