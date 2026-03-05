<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Props {
		open: boolean;
		title?: string;
		onClose: () => void;
		children: Snippet;
	}

	let { open, title, onClose, children }: Props = $props();

	function handleBackdrop(e: MouseEvent) {
		if (e.target === e.currentTarget) onClose();
	}

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === 'Escape') onClose();
	}
</script>

{#if open}
	<div
		class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4"
		role="dialog"
		aria-modal="true"
		tabindex="-1"
		onclick={handleBackdrop}
		onkeydown={handleKeydown}
	>
		<div class="w-full max-w-lg rounded-xl bg-surface-800 shadow-2xl">
			{#if title}
				<div class="flex items-center justify-between border-b border-surface-700 px-5 py-4">
					<h3 class="text-lg font-semibold text-surface-100">{title}</h3>
					<button
						class="rounded p-1 text-surface-400 transition hover:text-surface-100"
						onclick={onClose}
						aria-label="Close"
					>✕</button>
				</div>
			{/if}
			<div class="p-5">
				{@render children()}
			</div>
		</div>
	</div>
{/if}
