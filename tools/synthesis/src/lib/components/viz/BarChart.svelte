<script lang="ts">
	import type { ChartSeries } from '$lib/types/index.js';

	interface Props {
		series: ChartSeries[];
		title?: string;
		horizontal?: boolean;
	}

	let { series, title, horizontal = false }: Props = $props();

	const COLORS = ['#6366f1', '#22d3ee', '#f97316', '#10b981', '#f43f5e', '#a855f7', '#eab308'];

	let allPoints = $derived(series.flatMap((s) => s.data));
	let maxVal = $derived(Math.max(...allPoints.map((p) => p.value), 1));
	let labels = $derived([...new Set(allPoints.map((p) => p.label))]);
</script>

{#if title}
	<h2 class="mb-4 text-lg font-bold text-surface-100">{title}</h2>
{/if}

<div class="rounded-lg bg-surface-800 p-4">
	{#if horizontal}
		<div class="space-y-2">
			{#each labels as label, _li (label)}
				<div class="flex items-center gap-3">
					<span class="w-24 shrink-0 truncate text-right text-xs text-surface-400">{label}</span>
					<div class="flex flex-1 gap-1">
						{#each series as s, si (s.name)}
							{@const point = s.data.find((p) => p.label === label)}
							{#if point}
								<div
									class="h-6 rounded"
									style="width: {(point.value / maxVal) * 100}%; background: {s.color ?? point.color ?? COLORS[si % COLORS.length]}"
									title="{s.name}: {point.value}"
								></div>
							{/if}
						{/each}
					</div>
					<span class="w-12 text-right text-xs text-surface-400">
						{allPoints.find((p) => p.label === label)?.value ?? ''}
					</span>
				</div>
			{/each}
		</div>
	{:else}
		<svg viewBox="0 0 {labels.length * 60 + 40} 220" class="w-full" role="img">
			{#each labels as label, li (label)}
				{#each series as s, si (s.name)}
					{@const point = s.data.find((p) => p.label === label)}
					{#if point}
						{@const barW = 40 / series.length}
						{@const barH = (point.value / maxVal) * 180}
						<rect
							x={li * 60 + 20 + si * barW}
							y={200 - barH}
							width={barW - 2}
							height={barH}
							rx="2"
							fill={s.color ?? point.color ?? COLORS[si % COLORS.length]}
						>
							<title>{s.name}: {point.value}</title>
						</rect>
					{/if}
				{/each}
				<text
					x={li * 60 + 40}
					y="215"
					text-anchor="middle"
					class="fill-surface-400"
					font-size="10"
				>{label}</text>
			{/each}
		</svg>
	{/if}

	{#if series.length > 1}
		<div class="mt-3 flex flex-wrap gap-3">
			{#each series as s, si (s.name)}
				<div class="flex items-center gap-1.5 text-xs text-surface-400">
					<span class="inline-block h-2.5 w-2.5 rounded" style="background: {s.color ?? COLORS[si % COLORS.length]}"></span>
					{s.name}
				</div>
			{/each}
		</div>
	{/if}
</div>
