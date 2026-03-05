<script lang="ts">
	interface Props {
		value: number;
		max?: number;
		label?: string;
		color?: string;
	}

	let { value, max = 100, label, color = 'bg-accent-500' }: Props = $props();

	let pct = $derived(Math.min(100, Math.max(0, (value / max) * 100)));
</script>

<div class="space-y-1">
	{#if label}
		<div class="flex items-center justify-between text-xs">
			<span class="text-surface-300">{label}</span>
			<span class="text-surface-500">{Math.round(pct)}%</span>
		</div>
	{/if}
	<div class="h-2.5 w-full overflow-hidden rounded-full bg-surface-700">
		<div class="{color} h-full rounded-full transition-all" style="width: {pct}%"></div>
	</div>
</div>
