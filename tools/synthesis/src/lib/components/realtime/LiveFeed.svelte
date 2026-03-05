<script lang="ts">
	import type { ArticleItem } from '$lib/types/index.js';
	import ContentList from '../content/ContentList.svelte';
	import StatusIndicator from './StatusIndicator.svelte';

	interface Props {
		items: ArticleItem[];
		title?: string;
		connected?: boolean;
	}

	let { items, title, connected = true }: Props = $props();
</script>

<div class="space-y-3">
	<div class="flex items-center justify-between">
		{#if title}
			<h2 class="text-lg font-bold text-surface-100">{title}</h2>
		{/if}
		<StatusIndicator status={connected ? 'connected' : 'disconnected'} label={connected ? 'Live' : 'Offline'} />
	</div>
	<ContentList {items} />
</div>
