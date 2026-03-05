<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Props {
		left: Snippet;
		right: Snippet;
		ratio?: string;
	}

	let { left, right, ratio = '1fr 1fr' }: Props = $props();
</script>

<div class="grid gap-6" style="grid-template-columns: {ratio};">
	<div>
		{@render left()}
	</div>
	<div>
		{@render right()}
	</div>
</div>
