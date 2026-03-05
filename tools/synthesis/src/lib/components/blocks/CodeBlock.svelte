<script lang="ts">
	interface Props {
		code: string;
		language?: string;
		title?: string;
	}

	let { code, language, title }: Props = $props();

	let copied = $state(false);

	async function copy() {
		try {
			await navigator.clipboard.writeText(code);
			copied = true;
			setTimeout(() => (copied = false), 2000);
		} catch {
			// clipboard API unavailable
		}
	}
</script>

<div class="overflow-hidden rounded-lg bg-surface-800">
	<div class="flex items-center justify-between border-b border-surface-700 px-4 py-2">
		<span class="text-xs text-surface-500">
			{#if title}{title}{:else if language}{language}{:else}code{/if}
		</span>
		<button
			class="rounded px-2 py-1 text-xs text-surface-400 transition hover:bg-surface-700 hover:text-surface-200"
			onclick={copy}
		>
			{copied ? 'Copied!' : 'Copy'}
		</button>
	</div>
	<pre class="overflow-x-auto p-4 text-sm text-surface-200"><code>{code}</code></pre>
</div>
