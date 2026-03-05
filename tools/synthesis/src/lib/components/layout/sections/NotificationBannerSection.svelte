<script lang="ts">
	import NotificationBanner from '../../composite/NotificationBanner.svelte';

	interface Props {
		items: { message: string; type?: string; dismissible?: boolean }[];
	}

	let { items }: Props = $props();
</script>

{#each items as item (item.message)}
	<NotificationBanner
		message={item.message}
		type={(item.type ?? 'info') as 'info' | 'warning' | 'error' | 'success'}
		dismissible={item.dismissible}
	/>
{/each}
