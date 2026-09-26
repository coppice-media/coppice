<script lang="ts">
	/**
	 * The root error boundary. Unknown routes and failed loads land here,
	 * outside the signed-in shell, so the page carries its own way back.
	 */
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import FileQuestionMarkIcon from '@lucide/svelte/icons/file-question-mark';
	import { Button } from '@stump/ui/components/ui/button';
	import { EmptyState } from '@stump/ui/components/ui/empty';

	const TITLES: Record<number, string> = {
		401: 'Sign in required',
		403: 'Not allowed',
		404: 'Page not found'
	};
	const title = $derived(TITLES[page.status] ?? 'Something went wrong');
	// The router's own 404 message is just "Not Found", which the title already says.
	const description = $derived(
		page.status === 404 ? 'There is nothing at this address.' : page.error?.message || 'This page could not be shown.'
	);
</script>

<svelte:head>
	<title>{page.status} · Coppice ingest</title>
</svelte:head>

<div class="grid min-h-svh place-items-center p-6">
	<EmptyState {title} {description} class="w-full max-w-md">
		{#snippet icon()}<FileQuestionMarkIcon aria-hidden="true" />{/snippet}
		<Button href={resolve('/drop')}>Back to drop folder</Button>
		<Button variant="outline" onclick={() => location.reload()}>Reload</Button>
	</EmptyState>
</div>
