<script lang="ts">
	import { browser } from '$app/environment';
	import { resolve } from '$app/paths';
	import { createQuery, useQueryClient } from '@tanstack/svelte-query';
	import { Toaster } from 'svelte-sonner';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Button } from '@stump/ui/components/ui/button';
	import { Separator } from '@stump/ui/components/ui/separator';
	import { createEditorSession, setEditorSession } from '$lib/editor/session.svelte';
	import { LibrariesDocument } from '$lib/graphql/generated/graphql';
	import { MeDocument } from '@stump/ui/graphql/generated/graphql';
	import { request } from '@stump/ui/graphql/client';
	import {
		initialProgressState,
		progressReducer,
		startIngestProgressSubscription
	} from '$lib/ingest/progress';
	import { offsetPagination } from '$lib/ingest/helpers';

	let { children } = $props();

	const session = createEditorSession();
	setEditorSession(session);
	const queryClient = useQueryClient();
	const meQuery = createQuery(() => ({
		queryKey: ['me'],
		queryFn: () => request(MeDocument, {}),
		enabled: browser
	}));
	const librariesQuery = createQuery(() => ({
		queryKey: ['libraries'],
		queryFn: () => request(LibrariesDocument, { pagination: offsetPagination(100) }),
		enabled: browser
	}));

	$effect(() => {
		if (meQuery.data?.me) session.user = meQuery.data.me;
		const libraries = librariesQuery.data?.libraries.nodes;
		if (!libraries) return;
		session.libraries = libraries;
		if (!session.selectedLibraryId || !libraries.some((library) => library.id === session.selectedLibraryId)) {
			const saved = localStorage.getItem('stump-editor-library');
			session.selectedLibraryId =
				(saved && libraries.some((library) => library.id === saved) ? saved : libraries[0]?.id) ?? '';
		}
	});

	$effect(() => {
		if (!browser || !session.selectedLibraryId) return;
		const libraryId = session.selectedLibraryId;
		const afterEventId = session.progress.cursor;
		const unsubscribe = startIngestProgressSubscription({
			variables: {
				libraryId,
				dropItemId: null,
				analysisJobId: null,
				afterEventId
			},
			onEvent: (event) => {
				session.progress = progressReducer(session.progress, { type: 'EVENT', event });
			},
			onCursorExpired: () => {
				session.progress = progressReducer(session.progress, { type: 'CURSOR_EXPIRED' });
				void queryClient.invalidateQueries();
			},
			onError: (message) => {
				session.liveError = message;
			}
		});
		return () => unsubscribe();
	});

	function chooseLibrary(event: Event): void {
		const value = (event.currentTarget as HTMLSelectElement).value;
		session.selectedLibraryId = value;
		localStorage.setItem('stump-editor-library', value);
		session.progress = initialProgressState();
	}

	async function logout(): Promise<void> {
		await fetch('/api/v2/auth/logout', { method: 'POST', credentials: 'include' });
		window.location.assign(resolve('/login'));
	}
</script>

<svelte:head>
	<title>Stump · Ingest editor</title>
	<meta
		name="description"
		content="Review staged books, quality evidence, and metadata before adding them to Stump."
	/>
</svelte:head>

	<div class="min-h-screen bg-muted/30">
		<header class="border-b bg-background">
			<div class="mx-auto flex max-w-[1600px] flex-wrap items-center gap-4 px-4 py-4 lg:px-8">
				<a href={resolve('/drop')} class="mr-auto text-lg font-semibold tracking-tight">Stump ingest</a>
				<nav aria-label="Primary navigation" class="flex flex-wrap items-center gap-1 text-sm">
					<a class="rounded-md px-3 py-2 hover:bg-muted" href={resolve('/drop')}>Drop folder</a>
					<a class="rounded-md px-3 py-2 hover:bg-muted" href={resolve('/queue')}>Queue</a>
					<a class="rounded-md px-3 py-2 hover:bg-muted" href={resolve('/rework')}>Rework</a>
					<a class="rounded-md px-3 py-2 hover:bg-muted" href={resolve('/bulk')}>Bulk edit</a>
					<a class="rounded-md px-3 py-2 hover:bg-muted" href={resolve('/library')}>Library</a>
					<a class="rounded-md px-3 py-2 hover:bg-muted" href={resolve('/settings/providers')}>Settings</a>
				</nav>
				{#if session.libraries.length}
					<label class="flex items-center gap-2 text-sm">
						<span class="sr-only">Library</span>
						<select
							class="h-9 max-w-48 rounded-md border bg-background px-3 text-sm"
							value={session.selectedLibraryId}
						onchange={chooseLibrary}
						>
							{#each session.libraries as library (library.id)}
								<option value={library.id}>{library.emoji ? `${library.emoji} ` : ''}{library.name}</option>
							{/each}
						</select>
					</label>
				{/if}
				{#if session.user}
					<span class="hidden text-sm text-muted-foreground md:inline">{session.user.username}</span>
					<Button variant="ghost" size="sm" onclick={logout}>Log out</Button>
				{/if}
			</div>
		</header>
		<Separator />
		{#if session.liveError}
			<div class="mx-auto max-w-[1600px] px-4 pt-4 lg:px-8">
				<Alert variant="destructive">
					<AlertTitle>Live updates paused</AlertTitle>
					<AlertDescription>{session.liveError} Refresh the page to reconnect.</AlertDescription>
				</Alert>
			</div>
		{/if}
		<main class="mx-auto max-w-[1600px] px-4 py-8 lg:px-8">{@render children()}</main>
	</div>
	<Toaster position="bottom-right" />
