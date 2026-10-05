<script lang="ts">
	import { onMount, tick } from 'svelte';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@stump/ui/components/ui/card';
	import {
		CLUB_READER_MESSAGE_MAX_CHARS,
		ClubReaderApiError,
		deleteClubReaderMessage,
		listClubReaderMessages,
		postClubReaderMessage,
		updateClubReaderMessage,
		type ClubReaderMessage
	} from '$lib/club-reader-api';

	interface Props {
		sessionId: string;
		/** Participant fence sent on every write; the parent re-keys this component when it changes. */
		participantId: string;
		/** Currently published club queue row, attached to new messages. */
		bookId: string | null;
		/** Returns true when the failure meant this reader lost access (parent shows the unavailable state). */
		onAccessLost: (cause: unknown) => boolean;
		/** The published book changed under a post (409); the parent refreshes its snapshot. */
		onBookChanged: () => void;
	}

	let { sessionId, participantId, bookId, onAccessLost, onBookChanged }: Props = $props();

	const PAGE_SIZE = 50;
	const MAX_REFRESH_PAGES = 10;

	// Oldest first, so the newest message renders at the bottom.
	let messages = $state.raw<ClubReaderMessage[]>([]);
	let hasOlder = $state(false);
	let loaded = $state(false);
	let loadingOlder = $state(false);
	let error = $state<string | null>(null);
	let draft = $state('');
	let posting = $state(false);
	let editingId = $state<string | null>(null);
	let editDraft = $state('');
	let busyMessageId = $state<string | null>(null);
	let scroller = $state<HTMLElement | null>(null);

	let refreshing = false;
	let refreshAgain = false;
	let destroyed = false;
	const controller = new AbortController();

	const draftLength = $derived(Array.from(draft).length);

	/**
	 * Message-scoped statuses are not access loss: a 404 on edit/delete means the
	 * message was already removed (author or organizer), a 400 on "load older"
	 * means its cursor was deleted, a 409 means the published book changed. Each
	 * re-reads the list; real access loss then surfaces through that GET.
	 */
	function handleFailure(cause: unknown, scope: 'list' | 'older' | 'post' | 'message' = 'list'): void {
		if (destroyed || (cause instanceof Error && cause.name === 'AbortError')) return;
		const status = cause instanceof ClubReaderApiError ? cause.status : null;
		if (scope === 'message' && status === 404) {
			error = 'That message was already removed.';
			editingId = null;
			void refresh();
			return;
		}
		if (scope === 'older' && status === 400) {
			void refresh();
			return;
		}
		if (onAccessLost(cause)) return;
		if (scope === 'post' && status === 409) {
			error = 'The club’s published book changed. Your message was not sent; try again.';
			onBookChanged();
			return;
		}
		error =
			cause instanceof Error ? cause.message : 'The discussion could not be updated.';
	}

	function nearBottom(): boolean {
		if (!scroller) return true;
		return scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 48;
	}

	async function showMessages(next: ClubReaderMessage[], stickToBottom: boolean): Promise<void> {
		messages = next;
		await tick();
		if (stickToBottom && scroller) scroller.scrollTop = scroller.scrollHeight;
	}

	/**
	 * Re-reads newest pages until they cover every message already on screen, so
	 * edits and deletions (including organizer moderation) apply to loaded history.
	 */
	export async function refresh(): Promise<void> {
		if (refreshing) {
			refreshAgain = true;
			return;
		}
		refreshing = true;
		try {
			do {
				refreshAgain = false;
				const oldestShown = messages[0] ? Date.parse(messages[0].createdAt) : null;
				const collected: ClubReaderMessage[] = [];
				let before: string | undefined;
				let more = false;
				for (let pageIndex = 0; pageIndex < MAX_REFRESH_PAGES; pageIndex += 1) {
					const page = await listClubReaderMessages(sessionId, { before, limit: PAGE_SIZE }, controller.signal);
					collected.push(...page.messages);
					more = page.hasMore;
					const last = page.messages.at(-1);
					if (!more || !last || oldestShown === null || Date.parse(last.createdAt) <= oldestShown) break;
					before = last.id;
				}
				if (destroyed) return;
				const stick = !loaded || nearBottom();
				hasOlder = more;
				error = null;
				loaded = true;
				await showMessages(collected.reverse(), stick);
			} while (refreshAgain && !destroyed);
		} catch (cause) {
			handleFailure(cause);
		} finally {
			refreshing = false;
		}
	}

	async function loadOlder(): Promise<void> {
		const oldest = messages[0];
		if (!oldest || loadingOlder) return;
		loadingOlder = true;
		try {
			const page = await listClubReaderMessages(
				sessionId,
				{ before: oldest.id, limit: PAGE_SIZE },
				controller.signal
			);
			if (destroyed) return;
			const known = new Set(messages.map((message) => message.id));
			const older = page.messages.filter((message) => !known.has(message.id)).reverse();
			const previousHeight = scroller?.scrollHeight ?? 0;
			hasOlder = page.hasMore;
			messages = [...older, ...messages];
			await tick();
			if (scroller) scroller.scrollTop += scroller.scrollHeight - previousHeight;
		} catch (cause) {
			handleFailure(cause, 'older');
		} finally {
			loadingOlder = false;
		}
	}

	async function send(event: SubmitEvent): Promise<void> {
		event.preventDefault();
		const body = draft.trim();
		if (!body || posting) return;
		if (Array.from(body).length > CLUB_READER_MESSAGE_MAX_CHARS) {
			error = 'Messages are limited to 4,000 characters.';
			return;
		}
		posting = true;
		error = null;
		try {
			const created = await postClubReaderMessage(sessionId, participantId, {
				body,
				...(bookId ? { bookId } : {})
			});
			if (destroyed) return;
			draft = '';
			if (!messages.some((message) => message.id === created.id)) {
				await showMessages([...messages, created], true);
			}
		} catch (cause) {
			handleFailure(cause, 'post');
		} finally {
			posting = false;
		}
	}

	async function saveEdit(event: SubmitEvent, message: ClubReaderMessage): Promise<void> {
		event.preventDefault();
		const body = editDraft.trim();
		if (!body || busyMessageId) return;
		if (Array.from(body).length > CLUB_READER_MESSAGE_MAX_CHARS) {
			error = 'Messages are limited to 4,000 characters.';
			return;
		}
		busyMessageId = message.id;
		error = null;
		try {
			const updated = await updateClubReaderMessage(sessionId, message.id, participantId, body);
			if (destroyed) return;
			messages = messages.map((item) => (item.id === updated.id ? updated : item));
			editingId = null;
		} catch (cause) {
			handleFailure(cause, 'message');
		} finally {
			busyMessageId = null;
		}
	}

	async function remove(message: ClubReaderMessage): Promise<void> {
		if (busyMessageId || !window.confirm('Delete this message from the session discussion?')) return;
		busyMessageId = message.id;
		error = null;
		try {
			await deleteClubReaderMessage(sessionId, message.id, participantId);
			if (destroyed) return;
			messages = messages.filter((item) => item.id !== message.id);
			if (editingId === message.id) editingId = null;
		} catch (cause) {
			handleFailure(cause, 'message');
		} finally {
			busyMessageId = null;
		}
	}

	function formatTime(value: string): string {
		const date = new Date(value);
		return Number.isNaN(date.getTime())
			? ''
			: date.toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });
	}

	onMount(() => {
		void refresh();
		return () => {
			destroyed = true;
			controller.abort();
		};
	});
</script>

<section aria-labelledby="club-reader-discussion-heading">
	<Card>
		<CardHeader>
			<CardTitle id="club-reader-discussion-heading">Discussion</CardTitle>
			<CardDescription>
				Visible only to readers currently in this session, under their aliases. The club’s member
				discussions stay separate. Organizers can remove messages.
			</CardDescription>
		</CardHeader>
		<CardContent class="flex flex-col gap-4">
			{#if error}
				<p role="alert" class="text-sm text-destructive">{error}</p>
			{/if}

			{#if !loaded && error}
				<Button class="self-start" variant="outline" size="sm" onclick={() => void refresh()}>
					Retry loading the discussion
				</Button>
			{:else if !loaded}
				<p class="text-sm text-muted-foreground">Loading the discussion…</p>
			{:else}
				<div bind:this={scroller} class="flex max-h-[28rem] flex-col gap-3 overflow-y-auto pr-1">
					{#if hasOlder}
						<Button
							class="self-center"
							variant="ghost"
							size="sm"
							disabled={loadingOlder}
							onclick={() => void loadOlder()}
						>
							{loadingOlder ? 'Loading…' : 'Load older messages'}
						</Button>
					{/if}
					{#if messages.length}
						<ol class="flex flex-col gap-3" aria-label="Session discussion messages">
							{#each messages as message (message.id)}
								<li class={['rounded-lg border p-3', message.mine && 'bg-muted/40']}>
									<div class="flex flex-wrap items-center justify-between gap-2">
										<div class="flex min-w-0 flex-wrap items-center gap-2">
											<span class="truncate text-sm font-medium">{message.authorName}</span>
											{#if message.mine}
												<Badge variant="secondary">You</Badge>
											{/if}
											<time class="text-xs text-muted-foreground" datetime={message.createdAt}>
												{formatTime(message.createdAt)}
											</time>
											{#if message.editedAt}
												<span class="text-xs text-muted-foreground">(edited)</span>
											{/if}
											{#if message.bookId && message.bookId !== bookId}
												<span class="text-xs text-muted-foreground">· earlier book</span>
											{/if}
										</div>
										{#if message.mine && editingId !== message.id}
											<div class="flex gap-1">
												<Button
													variant="ghost"
													size="sm"
													disabled={busyMessageId !== null}
													onclick={() => {
														editingId = message.id;
														editDraft = message.body;
													}}
												>
													Edit
												</Button>
												<Button
													variant="ghost"
													size="sm"
													disabled={busyMessageId !== null}
													onclick={() => void remove(message)}
												>
													Delete
												</Button>
											</div>
										{/if}
									</div>
									{#if editingId === message.id}
										<form class="mt-2 flex flex-col gap-2" onsubmit={(event) => saveEdit(event, message)}>
											<label class="sr-only" for="discussion-edit-{message.id}">Edit message</label>
											<textarea
												id="discussion-edit-{message.id}"
												class="min-h-20 rounded-md border border-input bg-background p-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
												bind:value={editDraft}
												maxlength={CLUB_READER_MESSAGE_MAX_CHARS}
												required
											></textarea>
											<div class="flex gap-2">
												<Button type="submit" size="sm" disabled={busyMessageId !== null || !editDraft.trim()}>
													{busyMessageId === message.id ? 'Saving…' : 'Save'}
												</Button>
												<Button
													type="button"
													variant="ghost"
													size="sm"
													disabled={busyMessageId !== null}
													onclick={() => (editingId = null)}
												>
													Cancel
												</Button>
											</div>
										</form>
									{:else}
										<p class="mt-1 text-sm break-words whitespace-pre-wrap">{message.body}</p>
									{/if}
								</li>
							{/each}
						</ol>
					{:else}
						<p class="text-sm text-muted-foreground">No messages yet. Start the conversation.</p>
					{/if}
				</div>
			{/if}

			<form class="flex flex-col gap-2" onsubmit={send}>
				<label class="text-sm font-medium" for="club-reader-discussion-draft">Message</label>
				<textarea
					id="club-reader-discussion-draft"
					class="min-h-20 rounded-md border border-input bg-background p-3 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
					bind:value={draft}
					maxlength={CLUB_READER_MESSAGE_MAX_CHARS}
					placeholder="Share a thought with this session’s readers. Plain text only."
					disabled={!loaded}
				></textarea>
				<div class="flex flex-wrap items-center justify-between gap-2">
					<span class="text-xs text-muted-foreground tabular-nums">
						{draftLength} / {CLUB_READER_MESSAGE_MAX_CHARS}
					</span>
					<Button type="submit" disabled={!loaded || posting || !draft.trim()}>
						{posting ? 'Sending…' : 'Send'}
					</Button>
				</div>
			</form>
		</CardContent>
	</Card>
</section>
