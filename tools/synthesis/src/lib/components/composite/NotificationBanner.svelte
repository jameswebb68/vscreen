<script lang="ts">
	import type { NotificationType } from '$lib/types/index.js';

	interface Props {
		message: string;
		type?: NotificationType;
		dismissible?: boolean;
	}

	let { message, type = 'info', dismissible = false }: Props = $props();

	let visible = $state(true);

	const styles: Record<NotificationType, string> = {
		info: 'border-blue-500/40 bg-blue-500/10 text-blue-300',
		warning: 'border-yellow-500/40 bg-yellow-500/10 text-yellow-300',
		error: 'border-red-500/40 bg-red-500/10 text-red-300',
		success: 'border-green-500/40 bg-green-500/10 text-green-300',
	};

	const icons: Record<NotificationType, string> = {
		info: 'ℹ',
		warning: '⚠',
		error: '✕',
		success: '✓',
	};
</script>

{#if visible}
	<div class="flex items-center gap-3 rounded-lg border px-4 py-3 {styles[type]}">
		<span class="text-lg">{icons[type]}</span>
		<p class="flex-1 text-sm">{message}</p>
		{#if dismissible}
			<button
				class="rounded p-1 opacity-60 transition hover:opacity-100"
				onclick={() => (visible = false)}
				aria-label="Dismiss"
			>✕</button>
		{/if}
	</div>
{/if}
