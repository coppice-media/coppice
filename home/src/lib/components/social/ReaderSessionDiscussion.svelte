<script lang="ts">
	import { browser } from '$app/environment';
	import { createInfiniteQuery, useQueryClient } from '@tanstack/svelte-query';
	import { Button } from '@stump/ui/components/ui/button';
	import { readerSessionsApi, type ReaderSessionMessage, type ReaderSessionParticipant } from '$lib/reader-sessions';
	import { errorMessage } from '$lib/social';

	interface Props {
		clubId: string;
		sessionId: string;
		participants: readonly ReaderSessionParticipant[];
	}

	let { clubId, sessionId, participants }: Props = $props();

	const queryClient = useQueryClient();
	let open = $state(false);
	let deletingId = $state<string | null>(null);
	let deleteError = $state<string | null>(null);

	const messagesQuery = createInfiniteQuery(() => ({
		queryKey: ['book-club-reader-session-messages', clubId, sessionId],
		queryFn: ({ pageParam }: { pageParam: string | undefined }) =>
			readerSessionsApi.listMessages(clubId, sessionId, pageParam),
		initialPageParam: undefined as string | undefined,
		getNextPageParam: (lastPage) => (lastPage.hasMore ? lastPage.messages.at(-1)?.id : undefined),
		enabled: browser && open
	}));

	// Pages are newest-first; show the oldest at the top like the guest view.
	const messages = $derived(
		(messagesQuery.data?.pages ?? []).flatMap((page) => page.messages).reverse()
	);
	const participantsById = $derived(new Map(participants.map((participant) => [participant.id, participant])));

	async function moderate(message: ReaderSessionMessage): Promise<void> {
		if (deletingId || !window.confirm(`Delete this message by ${message.authorName} for every reader?`)) return;
		deletingId = message.id;
		deleteError = null;
		try {
			await readerSessionsApi.deleteMessage(clubId, sessionId, message.id);
		} catch (cause) {
			deleteError = errorMessage(cause, 'The message could not be deleted.');
		} finally {
			deletingId = null;
			// Refetch either way: a failed delete may mean someone else already removed it.
			await queryClient.invalidateQueries({ queryKey: ['book-club-reader-session-messages', clubId, sessionId] });
		}
	}

	function formatTime(value: string): string {
		const date = new Date(value);
		return Number.isNaN(date.getTime())
			? ''
			: date.toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });
	}
</script>

<details class="mt-3 rounded-md border" bind:open>
	<summary class="cursor-pointer px-3 py-2 text-sm font-medium">Discussion</summary>
	<div class="flex flex-col gap-3 border-t p-3">
		<p class="text-xs text-muted-foreground">
			Session-only messages, visible to this session’s active readers under their aliases. Deleting a
			message removes it for every reader immediately.
		</p>
		{#if deleteError}
			<p role="alert" class="text-sm text-destructive">{deleteError}</p>
		{/if}
		{#if messagesQuery.isPending}
			<p class="text-sm text-muted-foreground">Loading the discussion…</p>
		{:else if messagesQuery.isError}
			<p role="alert" class="text-sm text-destructive">{errorMessage(messagesQuery.error, 'The discussion could not be loaded.')}</p>
			<Button type="button" size="sm" variant="outline" class="self-start" onclick={() => void messagesQuery.refetch()}>
				Retry
			</Button>
		{:else}
			<div class="flex flex-wrap gap-2">
				{#if messagesQuery.hasNextPage}
					<Button
						type="button"
						size="sm"
						variant="ghost"
						disabled={messagesQuery.isFetchingNextPage}
						onclick={() => void messagesQuery.fetchNextPage()}
					>
						{messagesQuery.isFetchingNextPage ? 'Loading…' : 'Load older messages'}
					</Button>
				{/if}
				<Button
					type="button"
					size="sm"
					variant="outline"
					disabled={messagesQuery.isFetching}
					onclick={() => void messagesQuery.refetch()}
				>
					{messagesQuery.isFetching && !messagesQuery.isFetchingNextPage ? 'Refreshing…' : 'Refresh'}
				</Button>
			</div>
			{#if messages.length === 0}
				<p class="text-sm text-muted-foreground">No messages in this session.</p>
			{:else}
				<ol class="flex max-h-96 flex-col divide-y overflow-y-auto rounded-md border" aria-label="Session discussion messages">
					{#each messages as message (message.id)}
						{@const author = message.authorId ? participantsById.get(message.authorId) : undefined}
						<li class="flex items-start justify-between gap-3 px-3 py-2">
							<div class="min-w-0">
								<p class="text-xs text-muted-foreground">
									<span class="font-medium text-foreground">{message.authorName}</span>
									{#if author?.revokedAt}
										· revoked
									{/if}
									· <time datetime={message.createdAt}>{formatTime(message.createdAt)}</time>
									{#if message.editedAt}
										· edited
									{/if}
								</p>
								<p class="mt-1 text-sm break-words whitespace-pre-wrap">{message.body}</p>
							</div>
							<Button
								type="button"
								size="sm"
								variant="ghost"
								disabled={deletingId !== null}
								onclick={() => void moderate(message)}
							>
								{deletingId === message.id ? 'Deleting…' : 'Delete'}
							</Button>
						</li>
					{/each}
				</ol>
			{/if}
		{/if}
	</div>
</details>
