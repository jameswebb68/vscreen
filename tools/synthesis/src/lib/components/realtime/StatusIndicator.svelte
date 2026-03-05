<script lang="ts">
	interface Props {
		status: 'connected' | 'disconnected' | 'loading';
		label?: string;
	}

	let { status, label }: Props = $props();

	let dotClass = $derived(
		status === 'connected'
			? 'bg-green-400'
			: status === 'loading'
				? 'bg-yellow-400 animate-pulse'
				: 'bg-red-400'
	);

	let text = $derived(label ?? status);
</script>

<div class="flex items-center gap-2 text-xs text-surface-400">
	<span class="{dotClass} inline-block h-2 w-2 rounded-full"></span>
	<span class="capitalize">{text}</span>
</div>
