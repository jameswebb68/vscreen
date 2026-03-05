<script lang="ts">
	interface Props {
		title: string;
		subtitle?: string;
		backgroundImage?: string;
	}

	let { title, subtitle, backgroundImage }: Props = $props();
</script>

<section
	class="relative flex min-h-48 items-center justify-center overflow-hidden rounded-xl bg-surface-800 px-6 py-12 text-center"
	style={backgroundImage ? `background-image: url(${backgroundImage}); background-size: cover; background-position: center;` : ''}
>
	{#if backgroundImage}
		<div class="absolute inset-0 bg-black/60"></div>
	{/if}
	<div class="relative z-10">
		<h1 class="text-3xl font-bold text-white">{title}</h1>
		{#if subtitle}
			<p class="mt-2 text-lg text-surface-300">{subtitle}</p>
		{/if}
	</div>
</section>
