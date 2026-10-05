<script lang="ts">
	import { browser } from '$app/environment';
	import { createQuery, useQueryClient } from '@tanstack/svelte-query';
	import CrownIcon from '@lucide/svelte/icons/crown';
	import LogOutIcon from '@lucide/svelte/icons/log-out';
	import PlusIcon from '@lucide/svelte/icons/plus';
	import UserMinusIcon from '@lucide/svelte/icons/user-minus';
	import UserPlusIcon from '@lucide/svelte/icons/user-plus';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { Card, CardContent, CardDescription, CardFooter, CardHeader, CardTitle } from '@stump/ui/components/ui/card';
	import { Input } from '@stump/ui/components/ui/input';
	import { createLibrarySearch, SearchTerm, type LibraryHit } from '$lib/search.svelte';
	import { getHomeSession } from '$lib/session.svelte';
	import { readerSessionsApi, type ReaderSessionAdmin, type ReaderSessionCredential, type ReaderSessionParticipant } from '$lib/reader-sessions';
	import ReaderSessionDiscussion from '$lib/components/social/ReaderSessionDiscussion.svelte';
	import type { BookClub, BookClubInvitation } from '$lib/social';
	import { errorMessage } from '$lib/social';

	interface Props {
		clubs: readonly BookClub[];
		invitations: readonly BookClubInvitation[];
		canCreate: boolean;
		busy?: boolean;
		error?: string | null;
		onInvite?: (clubId: string, userId: string, role: string) => void;
		onRemove?: (clubId: string, memberId: string) => void;
		onLeave?: (clubId: string) => void;
		onRespondInvitation?: (invitation: BookClubInvitation, accept: boolean) => void;
		onCreate?: (input: { name: string; description: string; isPrivate: boolean; creatorHideProgress: boolean }) => void;
		onAddQueueBook?: (clubId: string, mediaId: string) => Promise<boolean>;
		onReorderQueue?: (clubId: string, bookIds: string[]) => Promise<boolean>;
		onCompleteQueueBook?: (clubId: string, bookClubBookId: string) => Promise<boolean>;
	}

	let {
		clubs,
		invitations,
		canCreate,
		busy = false,
		error = null,
		onInvite,
		onRemove,
		onLeave,
		onRespondInvitation,
		onCreate,
		onAddQueueBook,
		onReorderQueue,
		onCompleteQueueBook
	}: Props = $props();

	const session = getHomeSession();
	const queryClient = useQueryClient();
	const queueSearchTerm = new SearchTerm();
	const queueLibrarySearch = createLibrarySearch(() => queueSearchTerm.query, 6);
	let selectedClubId = $state<string | null>(null);
	let inviteUserId = $state('');
	let inviteRole = $state('MEMBER');
	let createName = $state('');
	let createDescription = $state('');
	let createPrivate = $state(true);
	let creatorHideProgress = $state(true);
	let selectedQueueBook = $state<LibraryHit | null>(null);
	let readerSessionName = $state('');
	let readerSessionExpiresAt = $state('');
	let accountJoinNames = $state<Record<string, string>>({});
	let accountJoinProgressOptIn = $state<Record<string, boolean>>({});
	let guestInviteSessionId = $state<string | null>(null);
	let guestDisplayName = $state('');
	let guestExpiresAt = $state('');
	let rotatingParticipantId = $state<string | null>(null);
	let rotateExpiresAt = $state('');
	let sessionBusyAction = $state<string | null>(null);
	let sessionError = $state<string | null>(null);
	let sessionNotice = $state<string | null>(null);
	let oneTimeCredential = $state<ReaderSessionCredential | null>(null);
	let oneTimeCredentialSessionName = $state('');
	let credentialCopyNotice = $state<string | null>(null);

	const selectedClub = $derived(
		clubs.find((club) => club.id === selectedClubId) ?? clubs[0] ?? null
	);
	const membershipRole = $derived(selectedClub?.membership?.role ?? '');
	const canManageMembers = $derived(membershipRole === 'ADMIN' || membershipRole === 'CREATOR');
	const canLeave = $derived(Boolean(selectedClub?.membership) && !selectedClub?.membership?.isCreator);
	const canManageQueue = $derived(Boolean(selectedClub?.membership) && canManageMembers);
	// Server owners bypass club roles and permissions on the server; mirror that
	// here so the panel never disables actions the API would accept.
	const isServerOwner = $derived(Boolean(session.user?.isServerOwner));
	const hasShareReaderPermission = $derived(
		isServerOwner ||
			Boolean(session.user?.permissions.some((permission) => String(permission) === 'SHARE_BOOK_CLUB_READER'))
	);
	const canCreateReaderSessions = $derived(isServerOwner || (canManageMembers && hasShareReaderPermission));
	const sessionActionsBusy = $derived(busy || sessionBusyAction !== null);
	const queueSearchResults = $derived(queueLibrarySearch.data?.librarySearch ?? []);
	const pendingQueueBooks = $derived(
		(selectedClub?.books ?? [])
			.filter((book) => !book.completedAt)
			.slice()
			.sort((left, right) => left.position - right.position)
	);
	const currentBook = $derived(selectedClub?.currentBook ?? pendingQueueBooks[0] ?? null);
	const upcomingBooks = $derived(
		pendingQueueBooks.filter((book) => book.id !== currentBook?.id)
	);
	const completedBooks = $derived(
		(selectedClub?.books ?? [])
			.filter((book) => Boolean(book.completedAt))
			.slice()
			.sort((left, right) => Date.parse(right.completedAt ?? '') - Date.parse(left.completedAt ?? ''))
	);
	const readerSessionsQuery = createQuery(() => {
		const clubId = selectedClub?.id;
		return {
			queryKey: ['book-club-reader-sessions', clubId],
			queryFn: () => {
				if (!clubId || !selectedClub?.membership) throw new Error('Join this BookClub to load its reader sessions.');
				return readerSessionsApi.list(clubId);
			},
			enabled: browser && Boolean(clubId && selectedClub?.membership)
		};
	});
	const readerSessions = $derived(readerSessionsQuery.data?.sessions ?? []);

	function bookTitle(book: { title?: string | null; entity?: { resolvedName?: string | null; name?: string | null } | null }): string {
		return book.title || book.entity?.resolvedName || book.entity?.name || 'Untitled book';
	}

	function bookAuthors(book: { author?: string | null; entity?: { metadata?: { writers?: string[] | null } | null } | null }): string {
		return book.author || book.entity?.metadata?.writers?.join(', ') || 'Author not provided';
	}

	function isReaderSessionExpired(readerSession: ReaderSessionAdmin): boolean {
		return Boolean(readerSession.expiresAt && Date.parse(readerSession.expiresAt) <= Date.now());
	}

	function isReaderSessionActive(readerSession: ReaderSessionAdmin): boolean {
		return !readerSession.closedAt && !isReaderSessionExpired(readerSession);
	}

	// `canManage` is the server's own authorization result for this session.
	function canManageReaderSession(readerSession: ReaderSessionAdmin): boolean {
		return readerSession.canManage;
	}

	function formatExpiry(value: string | null): string {
		if (!value) return 'No expiry';
		return new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' }).format(new Date(value));
	}

	function participantStatus(participant: ReaderSessionParticipant, readerSession: ReaderSessionAdmin): string {
		if (participant.revokedAt) return 'Revoked';
		if (readerSession.closedAt) return 'Session closed';
		if (isReaderSessionExpired(readerSession)) return 'Session expired';
		if (participant.expiresAt && Date.parse(participant.expiresAt) <= Date.now()) return 'Expired';
		return 'Active';
	}

	function readerLink(credential: ReaderSessionCredential): string {
		const url = new URL(credential.readerPath, window.location.origin);
		url.hash = `token=${encodeURIComponent(credential.token)}`;
		return url.toString();
	}

	function canIssueCredential(): boolean {
		if (!oneTimeCredential) return true;
		sessionNotice = 'Copy or dismiss the previous one-time invite before creating another; it cannot be recovered.';
		return false;
	}

	async function runSessionAction<T>(key: string, action: () => Promise<T>): Promise<T | null> {
		sessionBusyAction = key;
		sessionError = null;
		sessionNotice = null;
		try {
			return await action();
		} catch (caught) {
			sessionError = errorMessage(caught, 'The server rejected this reader-session action.');
			return null;
		} finally {
			sessionBusyAction = null;
		}
	}

	async function invalidateReaderSessions(clubId: string): Promise<void> {
		await Promise.all([
			queryClient.invalidateQueries({ queryKey: ['book-club-reader-sessions', clubId] }),
			queryClient.invalidateQueries({ queryKey: ['social-book-clubs'] })
		]);
	}

	function selectClub(clubId: string): void {
		if (sessionActionsBusy || selectedClub?.id === clubId) return;
		if (oneTimeCredential) {
			sessionNotice = 'Copy or dismiss the one-time invite before switching clubs; it cannot be recovered.';
			return;
		}
		selectedClubId = clubId;
		selectedQueueBook = null;
		queueSearchTerm.reset();
		readerSessionName = '';
		readerSessionExpiresAt = '';
		accountJoinNames = {};
		accountJoinProgressOptIn = {};
		guestInviteSessionId = null;
		guestDisplayName = '';
		guestExpiresAt = '';
		rotatingParticipantId = null;
		rotateExpiresAt = '';
		sessionError = null;
		sessionNotice = null;
	}

	function submitInvite(): void {
		const userId = inviteUserId.trim();
		if (!selectedClub || !canManageMembers || !userId) return;
		onInvite?.(selectedClub.id, userId, inviteRole);
		inviteUserId = '';
	}

	function submitCreate(): void {
		const name = createName.trim();
		if (!canCreate || !name) return;
		onCreate?.({
			name,
			description: createDescription.trim(),
			isPrivate: createPrivate,
			creatorHideProgress
		});
		createName = '';
		createDescription = '';
		createPrivate = true;
		creatorHideProgress = true;
	}

	async function addSelectedQueueBook(): Promise<void> {
		if (!selectedClub || !selectedQueueBook || !canManageQueue || busy || !onAddQueueBook) return;
		const added = await onAddQueueBook(selectedClub.id, selectedQueueBook.mediaId);
		if (added) {
			selectedQueueBook = null;
			queueSearchTerm.reset();
		}
	}

	async function reorderQueuedBook(bookId: string, direction: -1 | 1): Promise<void> {
		if (!selectedClub || !canManageQueue || busy || !onReorderQueue) return;
		const currentIndex = upcomingBooks.findIndex((book) => book.id === bookId);
		const nextIndex = currentIndex + direction;
		if (currentIndex < 0 || nextIndex < 0 || nextIndex >= upcomingBooks.length) return;
		const reorderedUpcoming = [...upcomingBooks];
		[reorderedUpcoming[currentIndex], reorderedUpcoming[nextIndex]] = [
			reorderedUpcoming[nextIndex],
			reorderedUpcoming[currentIndex]
		];
		const orderedIds = [
			...(currentBook ? [currentBook.id] : []),
			...reorderedUpcoming.map((book) => book.id)
		];
		await onReorderQueue(selectedClub.id, orderedIds);
	}

	async function completeCurrentBook(): Promise<void> {
		if (!selectedClub || !currentBook || !canManageQueue || busy || !onCompleteQueueBook) return;
		await onCompleteQueueBook(selectedClub.id, currentBook.id);
	}

	async function createReaderSession(): Promise<void> {
		const club = selectedClub;
		const name = readerSessionName.trim();
		if (!club || !canCreateReaderSessions || !name || sessionActionsBusy) return;
		const created = await runSessionAction('create-session', () => {
			const expiresAt = readerSessionExpiresAt ? new Date(readerSessionExpiresAt).toISOString() : undefined;
			return readerSessionsApi.create(club.id, { name, ...(expiresAt ? { expiresAt } : {}) });
		});
		if (!created) return;
		readerSessionName = '';
		readerSessionExpiresAt = '';
		sessionNotice = 'Reader session created.';
		await invalidateReaderSessions(club.id);
	}

	async function joinReaderSession(readerSession: ReaderSessionAdmin): Promise<void> {
		const club = selectedClub;
		const displayName = accountJoinNames[readerSession.id]?.trim() ?? '';
		if (!club?.membership || !isReaderSessionActive(readerSession) || !displayName || sessionActionsBusy) return;
		if (!canIssueCredential()) return;
		const credential = await runSessionAction(`join:${readerSession.id}`, () =>
			readerSessionsApi.join(club.id, readerSession.id, {
				displayName,
				shareProgress: accountJoinProgressOptIn[readerSession.id] ?? false
			})
		);
		if (!credential) return;
		oneTimeCredential = credential;
		oneTimeCredentialSessionName = readerSession.name;
		credentialCopyNotice = null;
		accountJoinNames = { ...accountJoinNames, [readerSession.id]: '' };
		accountJoinProgressOptIn = { ...accountJoinProgressOptIn, [readerSession.id]: false };
		await invalidateReaderSessions(club.id);
	}

	async function createGuestParticipant(readerSession: ReaderSessionAdmin): Promise<void> {
		const club = selectedClub;
		const displayName = guestDisplayName.trim();
		if (!club || !canManageReaderSession(readerSession) || !isReaderSessionActive(readerSession) || !displayName || sessionActionsBusy) return;
		if (!canIssueCredential()) return;
		const credential = await runSessionAction(`participant-create:${readerSession.id}`, () => {
			const expiresAt = guestExpiresAt ? new Date(guestExpiresAt).toISOString() : undefined;
			return readerSessionsApi.addParticipant(club.id, readerSession.id, {
				displayName,
				...(expiresAt ? { expiresAt } : {})
			});
		});
		if (!credential) return;
		oneTimeCredential = credential;
		oneTimeCredentialSessionName = readerSession.name;
		credentialCopyNotice = null;
		guestInviteSessionId = null;
		guestDisplayName = '';
		guestExpiresAt = '';
		sessionNotice = 'Guest invite created. Copy the link or token now; it will not be shown again.';
		await invalidateReaderSessions(club.id);
	}

	async function rotateParticipant(readerSession: ReaderSessionAdmin, participantId: string): Promise<void> {
		const club = selectedClub;
		if (!club || !canManageReaderSession(readerSession) || !isReaderSessionActive(readerSession) || sessionActionsBusy) return;
		if (!canIssueCredential()) return;
		const credential = await runSessionAction(`participant-rotate:${participantId}`, () => {
			const expiresAt = rotateExpiresAt ? new Date(rotateExpiresAt).toISOString() : undefined;
			return readerSessionsApi.rotateParticipant(club.id, readerSession.id, participantId, {
				...(expiresAt ? { expiresAt } : {})
			});
		});
		if (!credential) return;
		oneTimeCredential = credential;
		oneTimeCredentialSessionName = readerSession.name;
		credentialCopyNotice = null;
		rotatingParticipantId = null;
		rotateExpiresAt = '';
		sessionNotice = 'Invite rotated. Copy the new link or token now; the previous credential no longer works.';
		await invalidateReaderSessions(club.id);
	}

	async function revokeParticipant(readerSession: ReaderSessionAdmin, participantId: string, displayName: string): Promise<void> {
		const club = selectedClub;
		if (!club || !canManageReaderSession(readerSession) || readerSession.closedAt || sessionActionsBusy) return;
		if (!window.confirm(`Revoke the reader invite for “${displayName}”?`)) return;
		const result = await runSessionAction(`participant-revoke:${participantId}`, () =>
			readerSessionsApi.revokeParticipant(club.id, readerSession.id, participantId)
		);
		if (result === null) return;
		await invalidateReaderSessions(club.id);
	}

	async function closeReaderSession(readerSession: ReaderSessionAdmin): Promise<void> {
		const club = selectedClub;
		if (!club || !canManageReaderSession(readerSession) || readerSession.closedAt || sessionActionsBusy) return;
		if (!window.confirm(`Close “${readerSession.name}”? Existing reader links will stop working.`)) return;
		const result = await runSessionAction(`session-close:${readerSession.id}`, () =>
			readerSessionsApi.close(club.id, readerSession.id)
		);
		if (result === null) return;
		await invalidateReaderSessions(club.id);
	}

	async function publishCurrentBook(readerSession: ReaderSessionAdmin): Promise<void> {
		const club = selectedClub;
		if (
			!club ||
			!currentBook?.bookEntityId ||
			!canManageReaderSession(readerSession) ||
			!isReaderSessionActive(readerSession) ||
			sessionActionsBusy
		) return;
		const published = await runSessionAction(`publish:${readerSession.id}`, () =>
			readerSessionsApi.publish(club.id, readerSession.id, currentBook.id)
		);
		if (!published) return;
		sessionNotice = `Published “${bookTitle(currentBook)}” to “${readerSession.name}”.`;
		await invalidateReaderSessions(club.id);
	}

	async function advanceReaderSession(readerSession: ReaderSessionAdmin): Promise<void> {
		const club = selectedClub;
		if (
			!club ||
			!readerSession.publishedBookId ||
			readerSession.publishedBookId !== currentBook?.id ||
			!upcomingBooks[0]?.bookEntityId ||
			!canManageReaderSession(readerSession) ||
			!isReaderSessionActive(readerSession) ||
			sessionActionsBusy
		) return;
		const advanced = await runSessionAction(`advance:${readerSession.id}`, () =>
			readerSessionsApi.advance(club.id, readerSession.id)
		);
		if (!advanced) return;
		sessionNotice = 'The reader session advanced to the next eligible book.';
		await invalidateReaderSessions(club.id);
	}

	async function copyCredential(kind: 'link' | 'token'): Promise<void> {
		if (!oneTimeCredential || !browser) return;
		try {
			await navigator.clipboard.writeText(
				kind === 'link' ? readerLink(oneTimeCredential) : oneTimeCredential.token
			);
			credentialCopyNotice = kind === 'link' ? 'Reader link copied.' : 'Invite token copied.';
		} catch {
			credentialCopyNotice = 'Clipboard access was unavailable. Select and copy the displayed value.';
		}
	}
</script>


<div class="flex flex-col gap-4">
	{#if error}
		<Alert variant="destructive">
			<AlertTitle>BookClub action failed</AlertTitle>
			<AlertDescription>{error}</AlertDescription>
		</Alert>
	{/if}

	{#if invitations.length > 0}
		<Card>
			<CardHeader>
				<CardTitle class="text-base">Invitations</CardTitle>
				<CardDescription>Accept only clubs you recognize. Joining a club does not grant cross-user reading access.</CardDescription>
			</CardHeader>
			<CardContent class="flex flex-col gap-3">
				{#each invitations as invitation (invitation.id)}
					<div class="flex flex-wrap items-center justify-between gap-3 rounded-md border p-3">
						<div>
							<p class="font-medium">BookClub invitation</p>
							<p class="text-sm text-muted-foreground">Role: {invitation.role.toLowerCase()}</p>
						</div>
						<div class="flex flex-wrap gap-2">
							<Button type="button" size="sm" disabled={busy} onclick={() => onRespondInvitation?.(invitation, true)}>Accept</Button>
							<Button type="button" size="sm" variant="outline" disabled={busy} onclick={() => onRespondInvitation?.(invitation, false)}>Decline</Button>
						</div>
					</div>
				{/each}
			</CardContent>
		</Card>
	{/if}

	<div class="grid gap-4 @2xl/page:grid-cols-[minmax(14rem,18rem)_1fr]">
		<Card>
			<CardHeader>
				<CardTitle class="text-base">Your BookClubs</CardTitle>
				<CardDescription>Membership is explicit and separate from sharing grants.</CardDescription>
			</CardHeader>
			<CardContent class="flex flex-col gap-2">
				{#if clubs.length === 0}
					<p class="text-sm text-muted-foreground">You are not a member of a BookClub yet.</p>
				{:else}
					{#each clubs as club (club.id)}
						<button
							type="button"
							class="flex items-center justify-between rounded-md border px-3 py-2 text-left text-sm transition-colors hover:bg-accent {selectedClub?.id === club.id ? 'bg-accent' : ''}"
							aria-pressed={selectedClub?.id === club.id}
							disabled={sessionActionsBusy}
							onclick={() => selectClub(club.id)}
						>
							<span class="flex min-w-0 items-center gap-2">
								{#if club.emoji}<span aria-hidden="true">{club.emoji}</span>{/if}
								<span class="truncate">{club.name}</span>
							</span>
							<Badge variant="outline">{club.membersCount}</Badge>
						</button>
					{/each}
				{/if}
			</CardContent>
		</Card>

		{#if selectedClub}
			<Card>
				<CardHeader>
					<div class="flex flex-wrap items-start justify-between gap-3">
						<div>
							<CardTitle class="text-base">{selectedClub.name}</CardTitle>
							<CardDescription>{selectedClub.description || 'No description provided.'}</CardDescription>
						</div>
						<Badge variant="outline">{membershipRole.toLowerCase() || 'member'}</Badge>
					</div>
				</CardHeader>
				<CardContent class="flex flex-col gap-4">
					<div>
						<h3 class="text-sm font-medium">Members</h3>
						<ul class="mt-2 divide-y rounded-md border" aria-label="BookClub members">
							{#each selectedClub.members as member (member.id)}
								<li class="flex flex-wrap items-center justify-between gap-3 px-3 py-2 text-sm">
									<div class="flex min-w-0 items-center gap-2">
										{#if member.isCreator}<CrownIcon class="size-4 shrink-0" aria-label="Creator" />{/if}
										<span class="truncate">{member.displayName || member.username}</span>
										<Badge variant="secondary">{member.role.toLowerCase()}</Badge>
									</div>
									{#if canManageMembers && !member.isCreator && member.id !== selectedClub.membership?.id}
										<Button type="button" size="sm" variant="ghost" disabled={busy} aria-label="Remove {member.username}" onclick={() => onRemove?.(selectedClub.id, member.id)}>
											<UserMinusIcon class="size-4" />
										</Button>
									{/if}
								</li>
							{/each}
						</ul>
						<p class="mt-2 text-xs text-muted-foreground">Member progress remains hidden unless the member separately opts in through a scoped share grant.</p>
					</div>

					<section aria-labelledby="bookclub-queue-heading" class="flex flex-col gap-3">
						<div>
							<h3 id="bookclub-queue-heading" class="text-base font-semibold">Reading queue</h3>
							<p class="text-sm text-muted-foreground">The current book, upcoming picks, and completed reads are shared with this club.</p>
						</div>
						{#if currentBook}
							<div class="rounded-md border border-primary/40 bg-muted/20 p-3">
								<div class="flex flex-wrap items-start justify-between gap-3">
									<div class="min-w-0">
										<Badge variant="secondary">Current book</Badge>
										<p class="mt-2 font-medium">{bookTitle(currentBook)}</p>
										<p class="text-sm text-muted-foreground">{bookAuthors(currentBook)}</p>
									</div>
									{#if canManageQueue}
										<Button type="button" size="sm" variant="outline" disabled={busy || !onCompleteQueueBook} onclick={() => void completeCurrentBook()}>
											Mark completed
										</Button>
									{/if}
								</div>
							</div>
						{:else}
							<p class="rounded-md border border-dashed p-3 text-sm text-muted-foreground">No book is currently in the reading queue.</p>
						{/if}

						{#if upcomingBooks.length > 0}
							<div>
								<h4 class="text-sm font-medium">Up next</h4>
								<ol class="mt-2 flex flex-col gap-2" aria-label="Upcoming books">
									{#each upcomingBooks as book, index (book.id)}
										<li class="flex flex-wrap items-center justify-between gap-3 rounded-md border px-3 py-2">
											<div class="min-w-0">
												<p class="truncate text-sm font-medium">{bookTitle(book)}</p>
												<p class="truncate text-xs text-muted-foreground">{bookAuthors(book)}</p>
											</div>
											{#if canManageQueue}
												<div class="flex gap-1">
													<Button type="button" size="sm" variant="outline" disabled={busy || !onReorderQueue || index === 0} aria-label="Move {bookTitle(book)} up in the queue" onclick={() => void reorderQueuedBook(book.id, -1)}>Move up</Button>
													<Button type="button" size="sm" variant="outline" disabled={busy || !onReorderQueue || index === upcomingBooks.length - 1} aria-label="Move {bookTitle(book)} down in the queue" onclick={() => void reorderQueuedBook(book.id, 1)}>Move down</Button>
												</div>
											{/if}
										</li>
									{/each}
								</ol>
							</div>
						{/if}

						{#if completedBooks.length > 0}
							<details class="rounded-md border p-3">
								<summary class="cursor-pointer text-sm font-medium">Completed books ({completedBooks.length})</summary>
								<ol class="mt-3 flex flex-col gap-2" aria-label="Completed books">
									{#each completedBooks as book (book.id)}
										<li class="flex flex-wrap items-center justify-between gap-3 text-sm">
											<div class="min-w-0">
												<p class="truncate font-medium">{bookTitle(book)}</p>
												<p class="truncate text-xs text-muted-foreground">{bookAuthors(book)}</p>
											</div>
											<Badge variant="outline">Completed {book.completedAt ? formatExpiry(book.completedAt) : ''}</Badge>
										</li>
									{/each}
								</ol>
							</details>
						{/if}

						{#if canManageQueue}
							<div class="rounded-md border p-3">
								<h4 class="text-sm font-medium">Add a book from your library</h4>
								<p class="mt-1 text-xs text-muted-foreground">Search and choose a visible local book; only its title is shown here.</p>
								<label class="mt-3 block text-sm font-medium" for="bookclub-queue-library-search">Find a local book</label>
								<Input
									id="bookclub-queue-library-search"
									class="mt-1"
									placeholder="Type at least two characters"
									autocomplete="off"
									bind:value={() => queueSearchTerm.value, (value) => {
										queueSearchTerm.update(value);
										selectedQueueBook = null;
									}}
								/>
								{#if selectedQueueBook}
									<p class="mt-2 rounded-md bg-muted px-3 py-2 text-sm">
										Selected: <span class="font-medium">{selectedQueueBook.title}</span>
										{#if selectedQueueBook.authors}<span class="text-muted-foreground"> · {selectedQueueBook.authors}</span>{/if}
									</p>
								{:else if queueSearchTerm.query.length < 2}
									<p class="mt-2 text-xs text-muted-foreground">Enter at least two characters to search your library.</p>
								{:else if queueSearchTerm.typing || queueLibrarySearch.isPending}
									<p class="mt-2 text-xs text-muted-foreground">Searching your library…</p>
								{:else if queueLibrarySearch.isError}
									<p class="mt-2 text-xs text-destructive">{errorMessage(queueLibrarySearch.error, 'Library search is unavailable.')}</p>
								{:else if queueSearchResults.length === 0}
									<p class="mt-2 text-xs text-muted-foreground">No visible local books match this search.</p>
								{:else}
									<ul class="mt-2 flex max-h-56 flex-col gap-1 overflow-y-auto" aria-label="Local library search results">
										{#each queueSearchResults as hit (hit.mediaId)}
											<li>
												<button
													type="button"
													class="w-full rounded-md px-3 py-2 text-left transition-colors hover:bg-accent"
													onclick={() => {
														selectedQueueBook = hit;
														queueSearchTerm.reset(hit.title);
													}}
												>
													<span class="block truncate text-sm font-medium">{hit.title}</span>
													<span class="block truncate text-xs text-muted-foreground">{hit.authors || 'Author not provided'}{hit.seriesName ? ` · ${hit.seriesName}` : ''}</span>
												</button>
											</li>
										{/each}
									</ul>
								{/if}
								<Button type="button" size="sm" class="mt-3" disabled={busy || !onAddQueueBook || !selectedQueueBook} onclick={() => void addSelectedQueueBook()}>
									<PlusIcon class="mr-1 size-4" />
									Add to queue
								</Button>
							</div>
						{:else}
							<p class="rounded-md border border-dashed p-3 text-xs text-muted-foreground">Only club admins and creators can add books, reorder the queue, or mark the current book completed.</p>
						{/if}
					</section>

					<section aria-labelledby="reader-sessions-heading" class="flex flex-col gap-3 border-t pt-4">
						<div>
							<h3 id="reader-sessions-heading" class="text-base font-semibold">Guest-reader sessions</h3>
							<p class="text-sm text-muted-foreground">Publish one club book for a separate, read-only guest audience.</p>
						</div>
						<Alert>
							<AlertTitle>Guest-reader privacy</AlertTitle>
							<AlertDescription>
								Guests see only the published book and their own reading state. They do not see member usernames or a club roster, import account history, or access private annotations. Progress is private unless a member explicitly opts in.
							</AlertDescription>
						</Alert>

						{#if sessionError}
							<Alert variant="destructive">
								<AlertTitle>Reader-session action failed</AlertTitle>
								<AlertDescription>{sessionError}</AlertDescription>
							</Alert>
						{/if}
						{#if sessionNotice}
							<p class="rounded-md border border-primary/30 bg-primary/5 px-3 py-2 text-sm" role="status">{sessionNotice}</p>
						{/if}
						{#if oneTimeCredential}
							<div class="rounded-md border border-primary/40 bg-muted/20 p-3">
								<div class="flex flex-wrap items-start justify-between gap-3">
									<div>
										<h4 class="font-medium">One-time reader invite for {oneTimeCredential.participant.displayName} · {oneTimeCredentialSessionName}</h4>
										<p class="mt-1 text-xs text-muted-foreground">Expires: {formatExpiry(oneTimeCredential.participant.expiresAt)}. Copy the link or token now. The raw token is not saved and cannot be fetched again.</p>
									</div>
									<Button type="button" size="sm" variant="ghost" onclick={() => {
										oneTimeCredential = null;
										credentialCopyNotice = null;
										oneTimeCredentialSessionName = '';
									}}>Dismiss</Button>
								</div>
								<div class="mt-3 grid gap-2">
									<div>
										<label class="text-xs font-medium" for="reader-invite-link">Reader link</label>
										<div class="mt-1 flex flex-wrap gap-2">
											<Input id="reader-invite-link" class="min-w-48 flex-1" value={browser ? readerLink(oneTimeCredential) : ''} readonly autocomplete="off" />
											<Button type="button" size="sm" variant="outline" onclick={() => void copyCredential('link')}>Copy link</Button>
										</div>
									</div>
									<div>
										<label class="text-xs font-medium" for="reader-invite-token">One-time token</label>
										<div class="mt-1 flex flex-wrap gap-2">
											<Input id="reader-invite-token" class="min-w-48 flex-1 font-mono" value={oneTimeCredential.token} readonly autocomplete="off" />
											<Button type="button" size="sm" variant="outline" onclick={() => void copyCredential('token')}>Copy token</Button>
										</div>
									</div>
								</div>
								{#if credentialCopyNotice}<p class="mt-2 text-xs text-muted-foreground" role="status">{credentialCopyNotice}</p>{/if}
							</div>
						{/if}

						{#if canCreateReaderSessions}
							<div class="rounded-md border p-3">
								<h4 class="text-sm font-medium">Create a guest-reader session</h4>
								<p class="mt-1 text-xs text-muted-foreground">Organizer capability requires the SHARE_BOOK_CLUB_READER permission and an Admin or Creator club role.</p>
								<div class="mt-3 grid gap-3 @sm/page:grid-cols-2">
									<div>
										<label class="text-sm font-medium" for="reader-session-name">Session name</label>
										<Input id="reader-session-name" class="mt-1" bind:value={readerSessionName} placeholder="Saturday read-along" />
									</div>
									<div>
										<label class="text-sm font-medium" for="reader-session-expiry">Expires (optional)</label>
										<Input id="reader-session-expiry" class="mt-1" type="datetime-local" bind:value={readerSessionExpiresAt} />
									</div>
								</div>
								<Button type="button" size="sm" class="mt-3" disabled={sessionActionsBusy || !readerSessionName.trim()} onclick={() => void createReaderSession()}>
									Create session
								</Button>
							</div>
						{:else}
							<p class="rounded-md border border-dashed p-3 text-xs text-muted-foreground">
								Creating or managing a guest-reader session requires a club Admin or Creator role plus the SHARE_BOOK_CLUB_READER permission. Current role: {membershipRole || 'not a member'}; permission: {hasShareReaderPermission ? 'granted' : 'not granted'}.
							</p>
						{/if}

						{#if selectedClub.membership}
							{#if readerSessionsQuery.isPending}
								<p class="text-sm text-muted-foreground">Loading reader sessions…</p>
							{:else if readerSessionsQuery.isError}
								<Alert variant="destructive">
									<AlertTitle>Unable to load reader sessions</AlertTitle>
									<AlertDescription class="flex flex-wrap items-center justify-between gap-2">
										<span>{errorMessage(readerSessionsQuery.error, 'The server returned an error.')}</span>
										<Button type="button" size="sm" variant="outline" onclick={() => void readerSessionsQuery.refetch()}>Retry</Button>
									</AlertDescription>
								</Alert>
							{:else if readerSessions.length === 0}
								<p class="rounded-md border border-dashed p-3 text-sm text-muted-foreground">No reader sessions yet. An organizer can create one, then publish the current queued book.</p>
							{:else}
								<div class="flex flex-col gap-3">
									{#each readerSessions as readerSession (readerSession.id)}
										{@const sessionManager = canManageReaderSession(readerSession)}
										{@const sessionActive = isReaderSessionActive(readerSession)}
										<article class="rounded-md border p-3">
											<div class="flex flex-wrap items-start justify-between gap-3">
												<div class="min-w-0">
													<h4 class="font-medium">{readerSession.name}</h4>
													<p class="mt-1 text-xs text-muted-foreground">Expires: {formatExpiry(readerSession.expiresAt)}</p>
												</div>
												<Badge variant={readerSession.closedAt || isReaderSessionExpired(readerSession) ? 'outline' : 'secondary'}>
													{readerSession.closedAt ? 'Closed' : isReaderSessionExpired(readerSession) ? 'Expired' : 'Active'}
												</Badge>
											</div>
											<p class="mt-3 text-sm">
												{#if readerSession.publishedBookTitle}
													Published book: <span class="font-medium">{readerSession.publishedBookTitle}</span>
												{:else}
													No book is published to this session yet.
												{/if}
											</p>

											<div class="mt-3 flex flex-wrap gap-2">
												<Button
													type="button"
													size="sm"
													variant="outline"
													disabled={!sessionManager || !sessionActive || sessionActionsBusy || !currentBook?.bookEntityId || readerSession.publishedBookId === currentBook?.id}
													onclick={() => void publishCurrentBook(readerSession)}
												>
													Publish current book
												</Button>
												<Button
													type="button"
													size="sm"
													variant="outline"
													disabled={!sessionManager || !sessionActive || sessionActionsBusy || !readerSession.publishedBookId || readerSession.publishedBookId !== currentBook?.id || !upcomingBooks[0]?.bookEntityId}
													onclick={() => void advanceReaderSession(readerSession)}
												>
													Advance session
												</Button>
												<Button
													type="button"
													size="sm"
													variant="destructive"
													disabled={!sessionManager || Boolean(readerSession.closedAt) || sessionActionsBusy}
													onclick={() => void closeReaderSession(readerSession)}
												>
													Close session
												</Button>
											</div>
											{#if !sessionManager}
												<p class="mt-2 text-xs text-muted-foreground">Organizer actions are disabled unless this account has server canManage access, SHARE_BOOK_CLUB_READER, and an Admin or Creator role.</p>
											{:else if !currentBook}
												<p class="mt-2 text-xs text-muted-foreground">Add a local book to the queue before publishing a reader session.</p>
											{:else if !currentBook.bookEntityId}
												<p class="mt-2 text-xs text-muted-foreground">The current queue entry is not a local library book and cannot be published to a reader session.</p>
											{:else if readerSession.publishedBookId && readerSession.publishedBookId !== currentBook.id}
												<p class="mt-2 text-xs text-muted-foreground">The queue changed after this session was published. Publish the current book before advancing.</p>
											{:else if readerSession.publishedBookId === currentBook.id && !upcomingBooks[0]?.bookEntityId}
												<p class="mt-2 text-xs text-muted-foreground">Queue the next local library book before advancing this session. It can be completed without advancing from the session.</p>
											{/if}

											{#if selectedClub.membership}
												<div class="mt-4 rounded-md border-t pt-3">
													<h5 class="text-sm font-medium">Join this session as a reader</h5>
													<p class="mt-1 text-xs text-muted-foreground">Choose an alias. Your account history is not imported; sharing your progress is optional and off by default. Private annotations are never shared.</p>
													<div class="mt-2 grid gap-2 @sm/page:grid-cols-[1fr_auto]">
														<label class="sr-only" for="reader-session-alias-{readerSession.id}">Reader alias</label>
														<Input
															id="reader-session-alias-{readerSession.id}"
															placeholder="Choose a reader alias"
															autocomplete="off"
															disabled={!sessionActive || sessionActionsBusy}
															bind:value={() => accountJoinNames[readerSession.id] ?? '', (value) => {
																accountJoinNames = { ...accountJoinNames, [readerSession.id]: value };
															}}
														/>
														<Button
															type="button"
															size="sm"
															disabled={!sessionActive || sessionActionsBusy || !accountJoinNames[readerSession.id]?.trim()}
															onclick={() => void joinReaderSession(readerSession)}
														>
															Join session
														</Button>
													</div>
													<label class="mt-2 flex items-start gap-2 text-xs">
														<input
															type="checkbox"
															disabled={!sessionActive || sessionActionsBusy}
															bind:checked={() => accountJoinProgressOptIn[readerSession.id] ?? false, (checked) => {
																accountJoinProgressOptIn = { ...accountJoinProgressOptIn, [readerSession.id]: checked };
															}}
														/>
														<span>Explicitly share my progress with this book-club session.</span>
													</label>
												</div>
											{/if}

											{#if sessionManager}
												<div class="mt-4 rounded-md border-t pt-3">
													<div class="flex flex-wrap items-center justify-between gap-2">
														<h5 class="text-sm font-medium">Participants ({readerSession.participants.length})</h5>
														<Button
															type="button"
															size="sm"
															variant="outline"
															disabled={!sessionActive || sessionActionsBusy}
															onclick={() => {
																guestInviteSessionId = guestInviteSessionId === readerSession.id ? null : readerSession.id;
																guestDisplayName = '';
																guestExpiresAt = '';
															}}
														>
															Create guest invite
														</Button>
													</div>
													{#if guestInviteSessionId === readerSession.id}
														<div class="mt-3 rounded-md bg-muted/40 p-3">
															<label class="text-sm font-medium" for="guest-reader-alias">Guest alias</label>
															<Input id="guest-reader-alias" class="mt-1" bind:value={guestDisplayName} placeholder="Choose a guest name" autocomplete="off" />
															<label class="mt-3 block text-sm font-medium" for="guest-reader-expiry">Expires (optional)</label>
															<Input id="guest-reader-expiry" class="mt-1" type="datetime-local" bind:value={guestExpiresAt} />
															<div class="mt-3 flex flex-wrap gap-2">
																<Button type="button" size="sm" disabled={!guestDisplayName.trim() || !sessionActive || sessionActionsBusy} onclick={() => void createGuestParticipant(readerSession)}>Create one-time invite</Button>
																<Button type="button" size="sm" variant="ghost" disabled={sessionActionsBusy} onclick={() => (guestInviteSessionId = null)}>Cancel</Button>
															</div>
														</div>
													{/if}
													{#if readerSession.participants.length === 0}
														<p class="mt-2 text-xs text-muted-foreground">No participants have joined.</p>
													{:else}
														<ul class="mt-2 divide-y rounded-md border" aria-label="Reader-session participants">
															{#each readerSession.participants as participant (participant.id)}
																{@const participantState = participantStatus(participant, readerSession)}
																<li class="flex flex-wrap items-center justify-between gap-3 px-3 py-2">
																	<div class="min-w-0">
																		<p class="truncate text-sm font-medium">{participant.displayName}</p>
																		<p class="text-xs text-muted-foreground">{participant.linkedAccount ? 'Account-linked reader' : 'Guest reader'} · Expires: {formatExpiry(participant.expiresAt)}</p>
																	</div>
																	<div class="flex flex-wrap items-center gap-2">
																		<Badge variant={participantState === 'Active' ? 'secondary' : 'outline'}>{participantState}</Badge>
																		{#if rotatingParticipantId === participant.id}
																			<div class="flex flex-wrap items-center gap-2">
																				<label class="sr-only" for="rotate-reader-expiry-{participant.id}">New invite expiry</label>
																				<Input id="rotate-reader-expiry-{participant.id}" class="h-8 w-48" type="datetime-local" bind:value={rotateExpiresAt} />
																				<Button type="button" size="sm" disabled={!sessionActive || sessionActionsBusy || Boolean(participant.revokedAt)} onclick={() => void rotateParticipant(readerSession, participant.id)}>Rotate</Button>
																				<Button type="button" size="sm" variant="ghost" disabled={sessionActionsBusy} onclick={() => (rotatingParticipantId = null)}>Cancel</Button>
																			</div>
																		{:else}
																			<Button
																				type="button"
																				size="sm"
																				variant="outline"
																				disabled={!sessionActive || sessionActionsBusy || Boolean(participant.revokedAt)}
																				onclick={() => {
																					rotatingParticipantId = participant.id;
																					rotateExpiresAt = '';
																				}}
																			>
																				Rotate invite
																			</Button>
																		{/if}
																		<Button
																			type="button"
																			size="sm"
																			variant="ghost"
																			disabled={sessionActionsBusy || Boolean(participant.revokedAt) || Boolean(readerSession.closedAt)}
																			onclick={() => void revokeParticipant(readerSession, participant.id, participant.displayName)}
																		>
																			Revoke
																		</Button>
																	</div>
																</li>
															{/each}
														</ul>
													{/if}
													<ReaderSessionDiscussion
														clubId={selectedClub.id}
														sessionId={readerSession.id}
														participants={readerSession.participants}
													/>
												</div>
											{/if}
										</article>
									{/each}
								</div>
							{/if}
						{:else}
							<p class="rounded-md border border-dashed p-3 text-sm text-muted-foreground">Join this BookClub to view its reader sessions and explicitly join a session.</p>
						{/if}
					</section>

					{#if canManageMembers}
						<div class="rounded-md border p-3">
							<h3 class="text-sm font-medium">Invite a member</h3>
							<p class="mt-1 text-xs text-muted-foreground">Use the recipient's user ID from the narrow picker. Never paste a device or session identifier.</p>
							<div class="mt-3 grid gap-2 @sm/page:grid-cols-[1fr_auto]">
								<label class="sr-only" for="bookclub-invite-user">User ID</label>
								<Input id="bookclub-invite-user" bind:value={inviteUserId} placeholder="User ID" autocomplete="off" />
								<select class="h-9 rounded-md border bg-background px-3 text-sm" bind:value={inviteRole} aria-label="Invitation role">
									<option value="MEMBER">Member</option>
									<option value="MODERATOR">Moderator</option>
									<option value="ADMIN">Admin</option>
								</select>
							</div>
							<Button type="button" size="sm" class="mt-3" disabled={busy || !inviteUserId.trim()} onclick={submitInvite}>
								<UserPlusIcon class="mr-1 size-4" />
								Send invitation
							</Button>
						</div>
					{/if}
				</CardContent>
				{#if canLeave}
					<CardFooter class="border-t pt-4">
						<Button type="button" size="sm" variant="outline" disabled={busy} onclick={() => onLeave?.(selectedClub.id)}>
							<LogOutIcon class="mr-1 size-4" />
							Leave BookClub
						</Button>
					</CardFooter>
				{/if}
			</Card>
		{:else}
			<Card>
				<CardHeader>
					<CardTitle class="text-base">Start a BookClub</CardTitle>
					<CardDescription>{canCreate ? 'Create a private reading group and invite members explicitly.' : 'Ask a server owner for the Create BookClub permission.'}</CardDescription>
				</CardHeader>
				{#if canCreate}
					<CardContent class="flex flex-col gap-3">
						<div class="grid gap-2 @sm/page:grid-cols-2">
							<div>
								<label class="text-sm font-medium" for="bookclub-name">Name</label>
								<Input id="bookclub-name" class="mt-1" bind:value={createName} placeholder="Weekend reading" />
							</div>
							<div>
								<label class="text-sm font-medium" for="bookclub-description">Description</label>
								<Input id="bookclub-description" class="mt-1" bind:value={createDescription} placeholder="What are you reading?" />
							</div>
						</div>
						<label class="flex items-center gap-2 text-sm">
							<input type="checkbox" bind:checked={createPrivate} />
							<span>Keep this club private</span>
						</label>
						<label class="flex items-center gap-2 text-sm">
							<input type="checkbox" bind:checked={creatorHideProgress} />
							<span>Keep my progress hidden</span>
						</label>
					</CardContent>
					<CardFooter class="border-t pt-4">
						<Button type="button" size="sm" disabled={busy || !createName.trim()} onclick={submitCreate}>
							<PlusIcon class="mr-1 size-4" />
							Create BookClub
						</Button>
					</CardFooter>
				{/if}
			</Card>
		{/if}
	</div>
</div>
