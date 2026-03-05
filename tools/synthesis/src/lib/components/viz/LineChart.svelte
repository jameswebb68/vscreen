<script lang="ts">
	import type { ChartSeries } from '$lib/types/index.js';

	interface Props {
		series: ChartSeries[];
		title?: string;
		xLabel?: string;
		yLabel?: string;
	}

	let { series, title, xLabel, yLabel }: Props = $props();

	const COLORS = ['#6366f1', '#22d3ee', '#f97316', '#10b981', '#f43f5e', '#a855f7'];
	const W = 500;
	const H = 250;
	const PAD = { top: 10, right: 20, bottom: 30, left: 45 };
	let plotW = $derived(W - PAD.left - PAD.right);
	let plotH = $derived(H - PAD.top - PAD.bottom);

	let allPoints = $derived(series.flatMap((s) => s.data));
	let maxVal = $derived(Math.max(...allPoints.map((p) => p.value), 1));
	let labels = $derived(series[0]?.data.map((p) => p.label) ?? []);

	function pathForSeries(s: ChartSeries): string {
		return s.data
			.map((p, i) => {
				const x = PAD.left + (i / Math.max(s.data.length - 1, 1)) * plotW;
				const y = PAD.top + plotH - (p.value / maxVal) * plotH;
				return `${i === 0 ? 'M' : 'L'}${x},${y}`;
			})
			.join(' ');
	}
</script>

{#if title}
	<h2 class="mb-4 text-lg font-bold text-surface-100">{title}</h2>
{/if}

<div class="rounded-lg bg-surface-800 p-4">
	<svg viewBox="0 0 {W} {H}" class="w-full" role="img">
		<!-- Y-axis grid -->
		{#each [0, 0.25, 0.5, 0.75, 1] as frac (frac)}
			{@const y = PAD.top + plotH - frac * plotH}
			<line x1={PAD.left} y1={y} x2={W - PAD.right} y2={y} stroke="currentColor" class="text-surface-700" stroke-width="0.5" />
			<text x={PAD.left - 5} y={y + 3} text-anchor="end" font-size="9" class="fill-surface-500">
				{Math.round(maxVal * frac)}
			</text>
		{/each}

		<!-- X-axis labels -->
		{#each labels as label, i (label)}
			{@const x = PAD.left + (i / Math.max(labels.length - 1, 1)) * plotW}
			<text x={x} y={H - 5} text-anchor="middle" font-size="9" class="fill-surface-500">{label}</text>
		{/each}

		<!-- Lines -->
		{#each series as s, si (s.name)}
			<path
				d={pathForSeries(s)}
				fill="none"
				stroke={s.color ?? COLORS[si % COLORS.length]}
				stroke-width="2"
				stroke-linejoin="round"
			/>
			{#each s.data as p, i (p.label)}
				{@const x = PAD.left + (i / Math.max(s.data.length - 1, 1)) * plotW}
				{@const y = PAD.top + plotH - (p.value / maxVal) * plotH}
				<circle cx={x} cy={y} r="3" fill={s.color ?? COLORS[si % COLORS.length]}>
					<title>{s.name}: {p.label} = {p.value}</title>
				</circle>
			{/each}
		{/each}

		<!-- Axis labels -->
		{#if xLabel}
			<text x={PAD.left + plotW / 2} y={H} text-anchor="middle" font-size="10" class="fill-surface-400">{xLabel}</text>
		{/if}
		{#if yLabel}
			<text x="12" y={PAD.top + plotH / 2} text-anchor="middle" font-size="10" class="fill-surface-400" transform="rotate(-90, 12, {PAD.top + plotH / 2})">{yLabel}</text>
		{/if}
	</svg>

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
