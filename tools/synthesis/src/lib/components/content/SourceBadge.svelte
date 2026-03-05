<script lang="ts">
	interface Props {
		source: string;
		color?: string;
	}

	let { source, color }: Props = $props();

	const defaultColors: Record<string, string> = {
		cnn: 'bg-red-600',
		bbc: 'bg-amber-600',
		huffpost: 'bg-green-600',
		reuters: 'bg-orange-600',
		ap: 'bg-blue-600',
		nyt: 'bg-neutral-800'
	};

	let badgeClass = $derived(color ?? defaultColors[source.toLowerCase()] ?? 'bg-indigo-600');
</script>

<span class="{badgeClass} inline-block rounded px-2 py-0.5 text-xs font-bold text-white">
	{source}
</span>
