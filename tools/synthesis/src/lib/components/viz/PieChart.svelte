<script lang="ts">
	import type { ChartPoint } from '$lib/types/index.js';

	interface Props {
		data: ChartPoint[];
		title?: string;
		donut?: boolean;
	}

	let { data, title, donut = false }: Props = $props();

	const COLORS = ['#6366f1', '#22d3ee', '#f97316', '#10b981', '#f43f5e', '#a855f7', '#eab308', '#64748b'];
	const CX = 100;
	const CY = 100;
	const R = 85;
	let IR = $derived(donut ? 50 : 0);

	let total = $derived(data.reduce((sum, d) => sum + d.value, 0) || 1);

	let slices = $derived.by(() => {
		let angle = -Math.PI / 2;
		return data.map((d, i) => {
			const frac = d.value / total;
			const startAngle = angle;
			const endAngle = angle + frac * 2 * Math.PI;
			angle = endAngle;

			const x1 = CX + R * Math.cos(startAngle);
			const y1 = CY + R * Math.sin(startAngle);
			const x2 = CX + R * Math.cos(endAngle);
			const y2 = CY + R * Math.sin(endAngle);
			const largeArc = frac > 0.5 ? 1 : 0;

			let path = `M ${CX} ${CY} L ${x1} ${y1} A ${R} ${R} 0 ${largeArc} 1 ${x2} ${y2} Z`;
			if (donut) {
				const ix1 = CX + IR * Math.cos(startAngle);
				const iy1 = CY + IR * Math.sin(startAngle);
				const ix2 = CX + IR * Math.cos(endAngle);
				const iy2 = CY + IR * Math.sin(endAngle);
				path = `M ${x1} ${y1} A ${R} ${R} 0 ${largeArc} 1 ${x2} ${y2} L ${ix2} ${iy2} A ${IR} ${IR} 0 ${largeArc} 0 ${ix1} ${iy1} Z`;
			}

			return { d: path, color: d.color ?? COLORS[i % COLORS.length], label: d.label, value: d.value, pct: Math.round(frac * 100) };
		});
	});
</script>

{#if title}
	<h2 class="mb-4 text-lg font-bold text-surface-100">{title}</h2>
{/if}

<div class="flex flex-col items-center gap-4 rounded-lg bg-surface-800 p-4 md:flex-row">
	<svg viewBox="0 0 200 200" class="h-48 w-48 shrink-0" role="img">
		{#each slices as slice (slice.label)}
			<path d={slice.d} fill={slice.color}>
				<title>{slice.label}: {slice.value} ({slice.pct}%)</title>
			</path>
		{/each}
	</svg>

	<div class="flex flex-col gap-1.5">
		{#each slices as slice (slice.label)}
			<div class="flex items-center gap-2 text-xs text-surface-300">
				<span class="inline-block h-3 w-3 shrink-0 rounded" style="background: {slice.color}"></span>
				<span class="truncate">{slice.label}</span>
				<span class="ml-auto text-surface-500">{slice.pct}%</span>
			</div>
		{/each}
	</div>
</div>
