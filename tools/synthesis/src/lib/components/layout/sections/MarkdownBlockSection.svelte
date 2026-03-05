<script lang="ts">
	import MarkdownBlock from '../../blocks/MarkdownBlock.svelte';

	interface Props {
		items: { content: string }[];
		title?: string;
	}

	let { items, title }: Props = $props();

	let content = $derived(items[0]?.content ?? '');
</script>

<MarkdownBlock {content} {title} />
