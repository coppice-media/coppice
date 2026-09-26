<script lang="ts">
	/**
	 * The trailing control of a Hardcover hit: an *In library* link when the
	 * book is already here, the request's status when one exists (or was just
	 * created here), otherwise the format toggles and a *Request* button.
	 */
	import { createMutation, useQueryClient } from '@tanstack/svelte-query';
	import { resolve } from '$app/paths';
	import InboxIcon from '@lucide/svelte/icons/inbox';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { request } from '@stump/ui/graphql/client';
	import {
		CreateBookRequestDocument,
		type CreateBookRequestMutationVariables,
		type RequestFormat
	} from '$lib/graphql/generated/graphql';
	import { defaultRequestFormat, formatIncludesAudio, safeCoverUrl, statusLabel } from '$lib/requests';
	import { AUDIBLE_PROVIDER, type ExternalHit } from '$lib/search.svelte';
	import RequestFormatControl from './RequestFormatControl.svelte';

	let {
		hit,
		onnavigate
	}: {
		hit: ExternalHit;
		/** Fired when a badge link is followed, so a palette can close itself. */
		onnavigate?: () => void;
	} = $props();

	const queryClient = useQueryClient();
	// An Audible hit is one audiobook edition: its format is fixed and its
	// narrator seeds the preference. Anything else starts on what the
	// provider says exists. Rows are keyed by hit, so the seed is one-time.
	const audiobookOnly = $derived(hit.provider === AUDIBLE_PROVIDER);
	// svelte-ignore state_referenced_locally
	let format = $state<RequestFormat>(hit.provider === AUDIBLE_PROVIDER ? 'AUDIOBOOK' : defaultRequestFormat(hit));
	// svelte-ignore state_referenced_locally
	let narrator = $state<string | null>(hit.provider === AUDIBLE_PROVIDER ? (hit.narrators?.[0] ?? null) : null);

	const createRequest = createMutation(() => ({
		mutationFn: (input: CreateBookRequestMutationVariables['input']) =>
			request(CreateBookRequestDocument, { input }),
		onSuccess: () => {
			void queryClient.invalidateQueries({ queryKey: ['book-requests'] });
			// Cached hits still say “Request”; refetch them next time they are shown.
			void queryClient.invalidateQueries({ queryKey: ['external-book-search'], refetchType: 'none' });
		}
	}));
	const created = $derived(createRequest.data?.createBookRequest ?? null);
	const requestId = $derived(created?.id ?? hit.existingRequestId);
	const requestStatus = $derived(created?.status ?? hit.existingRequestStatus);

	function requestBook(): void {
		createRequest.mutate({
			external: {
				sourceProvider: hit.provider,
				remoteId: hit.remoteId,
				title: hit.title,
				authors: hit.authors,
				coverUrl: safeCoverUrl(hit.coverUrl)
			},
			isbn: hit.isbn13 || hit.isbn10 || null,
			format,
			preferredNarrator: formatIncludesAudio(format) ? narrator : null
		});
	}
</script>

{#if hit.inLibraryMediaIds.length}
	<Badge
		variant="secondary"
		href={resolve('/(app)/book/[mediaId]', { mediaId: hit.inLibraryMediaIds[0] })}
		aria-label={`Open ${hit.title} in your library`}
		onclick={() => onnavigate?.()}
	>
		In library
	</Badge>
{:else if requestId}
	<Badge
		variant="outline"
		href={resolve('/(app)/requests/[id]', { id: requestId })}
		aria-label={`Open the request for ${hit.title}`}
		onclick={() => onnavigate?.()}
	>
		<InboxIcon aria-hidden="true" />
		{statusLabel(requestStatus)}
	</Badge>
{:else}
	<div class="flex flex-col items-end gap-1">
		<div class="flex items-center gap-2">
			<RequestFormatControl
				bind:value={format}
				bind:narrator
				availability={hit}
				lookup={{ provider: hit.provider, remoteId: hit.remoteId, title: hit.title, authors: hit.authors }}
				{audiobookOnly}
				disabled={createRequest.isPending}
				label={`Request format for ${hit.title}`}
			/>
			<Button size="sm" onclick={requestBook} disabled={createRequest.isPending}>
				{createRequest.isPending ? 'Requesting…' : 'Request'}
			</Button>
		</div>
		{#if createRequest.error}
			<p class="text-xs text-destructive" role="alert">
				{createRequest.error instanceof Error ? createRequest.error.message : 'Unable to create request.'}
			</p>
		{/if}
	</div>
{/if}
