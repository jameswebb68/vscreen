<script lang="ts">
	import CodeBlock from '../../blocks/CodeBlock.svelte';

	interface Props {
		items: { code: string; language?: string }[];
		title?: string;
	}

	let { items, title }: Props = $props();

	let item = $derived(items[0]);
</script>

{#if item}
	<CodeBlock code={item.code} language={item.language} {title} />
{/if}
