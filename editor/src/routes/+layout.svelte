<script lang="ts">
	import './layout.css';
	import favicon from '$lib/assets/favicon.svg';
	import { initTheme } from '@stump/ui/theme.svelte.js';
	import QueryProvider from '$lib/components/QueryProvider.svelte';

	let { children } = $props();

	// `app.html` has already painted the stored theme; this keeps it in step
	// with the OS (mode `system`) and with the Home app, which writes the
	// same two localStorage keys on the same origin.
	$effect(() => initTheme());
</script>

<svelte:head><link rel="icon" href={favicon} /></svelte:head>
<QueryProvider>{@render children()}</QueryProvider>
