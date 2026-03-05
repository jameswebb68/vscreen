<script lang="ts">
	interface Props {
		text: string;
		author?: string;
		source?: string;
	}

	let { text, author, source }: Props = $props();
</script>

<blockquote class="rounded-lg border-l-4 border-accent-500 bg-surface-800 py-4 pl-5 pr-4">
	<p class="text-sm italic leading-relaxed text-surface-200">"{text}"</p>
	{#if author || source}
		<footer class="mt-3 text-xs text-surface-500">
			{#if author}
				<span class="font-medium text-surface-400">— {author}</span>
			{/if}
			{#if source}
				<span class="ml-1">({source})</span>
			{/if}
		</footer>
	{/if}
</blockquote>
