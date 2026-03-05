<script lang="ts">
	import QuoteBlock from '../../blocks/QuoteBlock.svelte';

	interface Props {
		items: { text: string; author?: string; source?: string }[];
	}

	let { items }: Props = $props();

	let item = $derived(items[0]);
</script>

{#if item}
	<QuoteBlock text={item.text} author={item.author} source={item.source} />
{/if}
