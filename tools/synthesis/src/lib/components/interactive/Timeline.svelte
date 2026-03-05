<script lang="ts">
	import type { TimelineEvent } from '$lib/types/index.js';

	interface Props {
		events: TimelineEvent[];
		title?: string;
	}

	let { events, title }: Props = $props();
</script>

{#if title}
	<h2 class="mb-4 text-lg font-bold text-surface-100">{title}</h2>
{/if}

<div class="relative ml-4 border-l-2 border-surface-700 pl-6">
	{#each events as event (event.date + event.title)}
		<div class="relative mb-6 last:mb-0">
			<div class="absolute -left-[31px] top-1 flex h-4 w-4 items-center justify-center rounded-full bg-surface-800 ring-2 ring-accent-500">
				{#if event.icon}
					<span class="text-xs">{event.icon}</span>
				{:else}
					<span class="h-2 w-2 rounded-full bg-accent-500"></span>
				{/if}
			</div>
			<time class="text-xs text-surface-500">{event.date}</time>
			<h3 class="mt-0.5 text-sm font-semibold text-surface-100">{event.title}</h3>
			{#if event.description}
				<p class="mt-1 text-sm text-surface-400">{event.description}</p>
			{/if}
		</div>
	{/each}
</div>
