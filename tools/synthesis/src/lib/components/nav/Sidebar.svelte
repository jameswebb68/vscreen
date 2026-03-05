<script lang="ts">
	import type { SidebarItem } from '$lib/types/index.js';

	interface Props {
		items: SidebarItem[];
		activeId?: string;
	}

	let { items, activeId }: Props = $props();
</script>

<nav class="w-56 shrink-0 rounded-lg bg-surface-800 p-3">
	<ul class="space-y-0.5">
		{#each items as item (item.id)}
			<li>
				{#if item.url}
					<a
						href={item.url}
						class="flex items-center gap-2 rounded-md px-3 py-2 text-sm transition
							{activeId === item.id ? 'bg-accent-500/20 text-accent-400 font-medium' : 'text-surface-300 hover:bg-surface-700 hover:text-surface-100'}"
					>
						{#if item.icon}
							<span class="text-base">{item.icon}</span>
						{/if}
						{item.label}
					</a>
				{:else}
					<span class="block px-3 py-2 text-xs font-semibold uppercase tracking-wider text-surface-500">
						{item.label}
					</span>
				{/if}
				{#if item.children && item.children.length > 0}
					<ul class="ml-4 space-y-0.5">
						{#each item.children as child (child.id)}
							<li>
								<a
									href={child.url ?? '#'}
									class="flex items-center gap-2 rounded-md px-3 py-1.5 text-sm transition
										{activeId === child.id ? 'text-accent-400 font-medium' : 'text-surface-400 hover:text-surface-200'}"
								>
									{#if child.icon}
										<span class="text-sm">{child.icon}</span>
									{/if}
									{child.label}
								</a>
							</li>
						{/each}
					</ul>
				{/if}
			</li>
		{/each}
	</ul>
</nav>
