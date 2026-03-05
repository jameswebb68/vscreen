<script lang="ts">
	interface Props {
		content: string;
		title?: string;
	}

	let { content, title }: Props = $props();

	let html = $derived(markdownToHtml(content));

	function markdownToHtml(md: string): string {
		let result = md
			.replace(/^### (.+)$/gm, '<h3>$1</h3>')
			.replace(/^## (.+)$/gm, '<h2>$1</h2>')
			.replace(/^# (.+)$/gm, '<h1>$1</h1>')
			.replace(/\*\*(.+?)\*\*/g, '<strong>$1</strong>')
			.replace(/\*(.+?)\*/g, '<em>$1</em>')
			.replace(/`(.+?)`/g, '<code>$1</code>')
			.replace(/\[(.+?)\]\((.+?)\)/g, '<a href="$2" target="_blank" rel="noopener noreferrer">$1</a>')
			.replace(/^- (.+)$/gm, '<li>$1</li>')
			.replace(/(<li>.*<\/li>\n?)+/g, '<ul>$&</ul>')
			.replace(/^(?!<[hulo]|<li)(.*\S.*)$/gm, '<p>$1</p>');
		return result;
	}
</script>

{#if title}
	<h2 class="mb-4 text-lg font-bold text-surface-100">{title}</h2>
{/if}

<!-- eslint-disable svelte/no-at-html-tags -- intentional: rendering user-supplied markdown as HTML -->
<div class="prose prose-invert max-w-none rounded-lg bg-surface-800 p-4">
	{@html html}
</div>
