<script lang="ts">
	import type { ArticleItem } from '$lib/types/index.js';
	import HeroSection from '../../content/HeroSection.svelte';

	interface Props {
		items: ArticleItem[];
	}

	let { items }: Props = $props();

	let heroData = $derived(items[0]);
</script>

{#if heroData}
	<HeroSection
		title={heroData.title}
		subtitle={heroData.description}
		backgroundImage={heroData.image}
	/>
{/if}
