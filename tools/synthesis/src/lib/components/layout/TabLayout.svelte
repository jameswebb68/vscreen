<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Tab {
		id: string;
		label: string;
	}

	interface Props {
		tabs: Tab[];
		children: Snippet<[string]>;
	}

	let { tabs, children }: Props = $props();

	let defaultTab = $derived(tabs[0]?.id ?? '');
	let activeTab = $state<string | null>(null);

	$effect(() => {
		if (activeTab === null) {
			activeTab = defaultTab;
		}
	});
</script>

<div>
	<div class="mb-4 flex gap-1 border-b border-surface-700">
		{#each tabs as tab (tab.id)}
			<button
				class="px-4 py-2 text-sm font-medium transition {activeTab === tab.id
					? 'border-b-2 border-accent-500 text-surface-100'
					: 'text-surface-400 hover:text-surface-200'}"
				onclick={() => (activeTab = tab.id)}
			>
				{tab.label}
			</button>
		{/each}
	</div>
	<div>
		{@render children(activeTab ?? '')}
	</div>
</div>
