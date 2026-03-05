<script lang="ts">
	import type { Section } from '$lib/types/index.js';
	import type { RegistryEntry } from './componentRegistry.js';
	import { componentRegistry } from './componentRegistry.js';

	interface Props {
		section: Section;
	}

	let { section }: Props = $props();

	let entry: RegistryEntry | undefined = $derived(componentRegistry[section.component]);
</script>

{#if entry}
	{@const Comp = entry.component}
	{@const compProps = entry.props(section)}
	<Comp {...compProps} />
{:else}
	<div class="rounded-lg bg-surface-800 p-4 text-sm text-surface-400">
		Unknown component: {section.component}
	</div>
{/if}
