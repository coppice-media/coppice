<script lang="ts">
	import './layout.css';
	import favicon from '$lib/assets/favicon.svg';
	import QueryProvider from '$lib/components/QueryProvider.svelte';

	let { children } = $props();
</script>

<svelte:head><link rel="icon" href={favicon} /></svelte:head>
<QueryProvider>{@render children()}</QueryProvider>
