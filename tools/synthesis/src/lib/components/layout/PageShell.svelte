<script lang="ts">
	import type { Snippet } from 'svelte';
	import type { ThemeType } from '$lib/types/index.js';

	interface Props {
		title: string;
		subtitle?: string;
		theme?: ThemeType;
		children: Snippet;
	}

	let { title, subtitle, theme = 'dark', children }: Props = $props();
</script>

<div class="{theme === 'dark' ? 'dark bg-surface-950 text-surface-100' : 'bg-white text-surface-900'} min-h-screen">
	<div class="mx-auto max-w-6xl p-6 md:p-8">
		<header class="mb-8">
			<h1 class="text-2xl font-bold">{title}</h1>
			{#if subtitle}
				<p class="mt-1 text-sm text-surface-400">{subtitle}</p>
			{/if}
		</header>
		<main>
			{@render children()}
		</main>
	</div>
</div>
