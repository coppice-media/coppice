<script lang="ts">
	import { browser } from '$app/environment';
	import { createQuery, useQueryClient } from '@tanstack/svelte-query';
	import CheckIcon from '@lucide/svelte/icons/check';
	import LightbulbIcon from '@lucide/svelte/icons/lightbulb';
	import SendIcon from '@lucide/svelte/icons/send';
	import Share2Icon from '@lucide/svelte/icons/share-2';
	import XIcon from '@lucide/svelte/icons/x';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Button } from '@stump/ui/components/ui/button';
	import { Card, CardContent, CardDescription, CardFooter, CardHeader, CardTitle } from '@stump/ui/components/ui/card';
	import { Cover } from '@stump/ui/components/ui/cover';
	import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from '@stump/ui/components/ui/empty';
	import { Input } from '@stump/ui/components/ui/input';
	import { PageHeader } from '@stump/ui/components/ui/page-header';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import RecommendationCard from '$lib/components/social/RecommendationCard.svelte';
	import ShareGrantCard from '$lib/components/social/ShareGrantCard.svelte';
	import SharedOverlayCard from '$lib/components/social/SharedOverlayCard.svelte';
	import BookClubPanel from '$lib/components/social/BookClubPanel.svelte';
	import { getHomeSession } from '$lib/session.svelte';
	import {
		AdaptiveRecommendationsDocument,
		BookClubsDocument,
		CreateBookClubDocument,
		CreateBookClubInvitationDocument,
		CreateShareGrantDocument,
		DismissRecommendationDocument,
		LeaveBookClubDocument,
		MyBookClubInvitationsDocument,
		RemoveBookClubMemberDocument,
		RespondToBookClubInvitationDocument,
		RespondToRecommendationDocument,
		RespondToShareGrantDocument,
		RevokeRecommendationDocument,
		RequestRecommendationDocument,
		RevokeShareGrantDocument,
		SendRecommendationDocument,
		SetOverlayVisibilityDocument,
		SetRecommendationOptOutDocument,
		SocialRequestDestinationsDocument,
		SocialShareOverlaysDocument,
		SocialShareGrantsDocument,
		SocialPreferencesDocument,
		IncomingSocialRecommendationsDocument,
		OutgoingSocialRecommendationsDocument,
		SocialRecipientsDocument,
		LISEUR_COLORS,
		SOCIAL_SCOPES,
		errorMessage,
		formatAuthors,
		socialRequest,
		type AdaptiveRecommendation,
		type BookClub,
		type BookClubInvitation,
		type SocialRecommendation,
		type SocialRecipient,
		type SocialShareGrant,
		type SocialShareOverlay,
		type ShareScope
	} from '$lib/social';

	const session = getHomeSession();
	const queryClient = useQueryClient();

	function loadAdaptiveDismissed(): Set<string> {
		if (!browser) return new Set();
		try {
			const saved = JSON.parse(localStorage.getItem('coppice.social.dismissedAdaptive') ?? '[]');
			return new Set(Array.isArray(saved) ? saved.filter((value): value is string => typeof value === 'string') : []);
		} catch {
			return new Set();
		}
	}

	let actionError = $state<string | null>(null);
	let busyAction = $state<string | null>(null);
	let localSentRecommendations = $state<SocialRecommendation[]>([]);
	let localSentShares = $state<SocialShareGrant[]>([]);
	let adaptiveDismissed = $state<Set<string>>(loadAdaptiveDismissed());
	let optOutDraft = $state<boolean | null>(null);
	let pendingRequestRecommendation = $state<SocialRecommendation | null>(null);
	let requestDestinationShelfId = $state('');
	let requestDestinationDeviceId = $state('');
	let requestedRecommendationIds = $state<Set<string>>(new Set());
	let recommendationUpdates = $state<Record<string, SocialRecommendation>>({});

	let recipientSearch = $state('');
	let selectedRecipient = $state<SocialRecipient | null>(null);
	let targetKind = $state<'INTERNAL_MEDIA' | 'INTERNAL_WORK' | 'EXTERNAL_WORK'>('EXTERNAL_WORK');
	let targetId = $state('');
	let recommendationTitle = $state('');
	let recommendationAuthors = $state('');
	let recommendationProvider = $state('');
	let recommendationRemoteId = $state('');
	let recommendationExternalKey = $state('');
	let recommendationCoverUrl = $state('');
	let recommendationMessage = $state('');

	let shareTargetKey = $state('');
	let shareTitle = $state('');
	let shareAuthors = $state('');
	let shareProvider = $state('');
	let shareRemoteId = $state('');
	let shareExternalKey = $state('');
	let shareScopes = $state<Set<ShareScope>>(new Set(['METADATA']));
	let shareExpiresAt = $state('');
	const incomingRecommendationsQuery = createQuery(() => ({
		queryKey: ['social-recommendations', 'incoming'],
		queryFn: () => socialRequest(IncomingSocialRecommendationsDocument),
		enabled: browser
	}));
	const outgoingRecommendationsQuery = createQuery(() => ({
		queryKey: ['social-recommendations', 'outgoing'],
		queryFn: () => socialRequest(OutgoingSocialRecommendationsDocument),
		enabled: browser
	}));
	const adaptiveQuery = createQuery(() => ({
		queryKey: ['adaptive-recommendations'],
		queryFn: () => socialRequest(AdaptiveRecommendationsDocument, { limit: 20 }),
		enabled: browser
	}));
	const preferencesQuery = createQuery(() => ({
		queryKey: ['social-preferences'],
		queryFn: () => socialRequest(SocialPreferencesDocument),
		enabled: browser
	}));
	const recipientsQuery = createQuery(() => ({
		queryKey: ['social-recipient-search', recipientSearch.trim()],
		queryFn: () => socialRequest(SocialRecipientsDocument, { query: recipientSearch.trim() }),
		enabled: browser && recipientSearch.trim().length >= 2 && !selectedRecipient
	}));
	const incomingSharesQuery = createQuery(() => ({
		queryKey: ['social-shares', 'incoming'],
		queryFn: () => socialRequest(SocialShareGrantsDocument, { incoming: true }),
		enabled: browser
	}));
	const participantSharesQuery = createQuery(() => ({
		queryKey: ['social-shares', 'outgoing'],
		queryFn: () => socialRequest(SocialShareGrantsDocument, { incoming: false }),
		enabled: browser
	}));
	const overlaysQuery = createQuery(() => ({
		queryKey: ['social-overlays'],
		queryFn: () => socialRequest(SocialShareOverlaysDocument, {}),
		enabled: browser
	}));
	const clubsQuery = createQuery(() => ({
		queryKey: ['social-book-clubs'],
		queryFn: () => socialRequest(BookClubsDocument, { all: false }),
		enabled: browser
	}));
	const invitationsQuery = createQuery(() => ({
		queryKey: ['social-book-club-invitations'],
		queryFn: () => socialRequest(MyBookClubInvitationsDocument),
		enabled: browser
	}));
	const destinationsQuery = createQuery(() => ({
		queryKey: ['social-request-destinations'],
		queryFn: () => socialRequest(SocialRequestDestinationsDocument),
		enabled: browser
	}));

	const incomingRecommendations = $derived(
		(incomingRecommendationsQuery.data?.socialRecommendations ?? []).map(
			(recommendation) => recommendationUpdates[recommendation.id] ?? recommendation
		)
	);
	const sentRecommendations = $derived(
		[...localSentRecommendations, ...(outgoingRecommendationsQuery.data?.socialRecommendations ?? [])].filter(
			(recommendation, index, recommendations) =>
				recommendations.findIndex((candidate) => candidate.id === recommendation.id) === index
		)
	);
	const adaptiveRecommendations = $derived(adaptiveQuery.data?.adaptiveRecommendations ?? []);
	const incomingShares = $derived(incomingSharesQuery.data?.socialShareGrants ?? []);
	const participantShares = $derived(participantSharesQuery.data?.socialShareGrants ?? []);
	const overlays = $derived(overlaysQuery.data?.socialOverlays ?? []);
	const clubs = $derived(clubsQuery.data?.bookClubs ?? []);
	const invitations = $derived(invitationsQuery.data?.myBookClubInvitations ?? []);
	const preferences = $derived(preferencesQuery.data?.socialPreferences ?? null);

	const requestDevices = $derived(
		(destinationsQuery.data?.devices ?? [])
			.filter((device) => !device.revokedAt)
			.map((device) => ({ id: device.id, name: device.name }))
	);
	const requestShelves = $derived(
		(destinationsQuery.data?.readingLists?.nodes ?? []).map((shelf) => ({ id: shelf.id, name: shelf.name }))
	);
	const sentShares = $derived(
		[...localSentShares, ...participantShares].filter(
			(grant, index, grants) => grants.findIndex((candidate) => candidate.id === grant.id) === index
		)
	);


	const recommendationOptOut = $derived(
		optOutDraft ?? preferences?.recommendationsOptOut ?? false
	);
	const sharingOptOut = $derived(preferences?.sharingOptOut ?? false);
	const visibleAdaptiveRecommendations = $derived(
		adaptiveRecommendations.filter((recommendation) => !adaptiveDismissed.has(recommendation.targetKey))
	);
	const canCreateBookClub = $derived(
		Boolean(
			session.user?.isServerOwner ||
			session.user?.permissions.includes('CREATE_BOOK_CLUB')
		)
	);

	async function runAction<T>(key: string, action: () => Promise<T>): Promise<T | null> {
		busyAction = key;
		actionError = null;
		try {
			return await action();
		} catch (error) {
			actionError = errorMessage(error, 'The server rejected this action.');
			return null;
		} finally {
			busyAction = null;
		}
	}

	async function invalidateSocial(): Promise<void> {
		await Promise.all([
			queryClient.invalidateQueries({ queryKey: ['social-recommendations'] }),
			queryClient.invalidateQueries({ queryKey: ['adaptive-recommendations'] }),
			queryClient.invalidateQueries({ queryKey: ['social-shares'] }),
			queryClient.invalidateQueries({ queryKey: ['social-overlays'] }),
			queryClient.invalidateQueries({ queryKey: ['social-preferences'] }),
			queryClient.invalidateQueries({ queryKey: ['social-book-club-invitations'] }),
			queryClient.invalidateQueries({ queryKey: ['social-book-clubs'] })
		]);
	}

	function clearRecommendationTarget(): void {
		targetId = '';
		recommendationTitle = '';
		recommendationAuthors = '';
		recommendationProvider = '';
		recommendationRemoteId = '';
		recommendationExternalKey = '';
		recommendationCoverUrl = '';
	}

	function clearRecipient(): void {
		selectedRecipient = null;
		recipientSearch = '';
	}

	function selectRecipient(recipient: SocialRecipient): void {
		selectedRecipient = recipient;
		recipientSearch = '';
	}

	function prepareAdaptiveRecommendation(recommendation: AdaptiveRecommendation): void {
		targetKind = 'EXTERNAL_WORK';
		targetId = '';
		recommendationTitle = recommendation.title;
		recommendationAuthors = formatAuthors(recommendation.authors);
		recommendationExternalKey = recommendation.targetKey;
		recommendationCoverUrl = recommendation.coverUrl ?? '';
		document.getElementById('recommendation-form')?.scrollIntoView({ behavior: 'smooth', block: 'start' });
	}

	function dismissAdaptiveRecommendation(recommendation: AdaptiveRecommendation): void {
		adaptiveDismissed = new Set(adaptiveDismissed).add(recommendation.targetKey);
		if (browser) {
			localStorage.setItem('coppice.social.dismissedAdaptive', JSON.stringify([...adaptiveDismissed]));
		}
	}

	function toggleScope(scope: ShareScope): void {
		const next = new Set(shareScopes);
		if (next.has(scope)) next.delete(scope);
		else next.add(scope);
		shareScopes = next;
	}

	async function toggleRecommendationOptOut(): Promise<void> {
		const next = !recommendationOptOut;
		optOutDraft = next;
		const result = await runAction('recommendation-opt-out', () =>
			socialRequest(SetRecommendationOptOutDocument, { optOut: next })
		);
		if (!result) {
			optOutDraft = null;
			return;
		}
		optOutDraft = result.setRecommendationOptOut.recommendationsOptOut;
		await queryClient.invalidateQueries({ queryKey: ['adaptive-recommendations'] });
		await queryClient.invalidateQueries({ queryKey: ['social-preferences'] });
	}

	async function sendRecommendation(): Promise<void> {
		if (!selectedRecipient) {
			actionError = 'Choose a recipient from the active-user search before sending.';
			return;
		}
		const title = recommendationTitle.trim();
		const authors = recommendationAuthors.trim();
		if (!title || !authors) {
			actionError = 'Add a title and author before sending a recommendation.';
			return;
		}
		if (targetKind !== 'EXTERNAL_WORK' && !targetId.trim()) {
			actionError = 'Add the visible library media or work ID for this recommendation.';
			return;
		}
		const input: Record<string, unknown> = {
			recipientUserId: selectedRecipient.id,
			targetKind,
			title,
			authors,
			message: recommendationMessage.trim() || null
		};
		if (targetKind === 'INTERNAL_MEDIA') input.mediaId = targetId.trim();
		if (targetKind === 'INTERNAL_WORK') input.workId = targetId.trim();
		if (recommendationProvider.trim()) input.sourceProvider = recommendationProvider.trim();
		if (recommendationRemoteId.trim()) input.remoteId = recommendationRemoteId.trim();
		if (recommendationExternalKey.trim()) input.externalKey = recommendationExternalKey.trim();
		if (recommendationCoverUrl.trim()) input.coverUrl = recommendationCoverUrl.trim();
		const result = await runAction('send-recommendation', () =>
			socialRequest(SendRecommendationDocument, { input })
		);
		if (!result) return;
		localSentRecommendations = [...localSentRecommendations, result.sendRecommendation];
		clearRecommendationTarget();
		recommendationMessage = '';
		clearRecipient();
		await invalidateSocial();
	}


	async function respondToRecommendation(recommendation: SocialRecommendation, accept: boolean): Promise<void> {
		const result = await runAction(`recommendation-response:${recommendation.id}`, () =>
			socialRequest(RespondToRecommendationDocument, {
				id: recommendation.id,
				response: { accept }
			})
		);
		if (!result) return;
		const updatedRecommendation = result.respondToRecommendation;
		recommendationUpdates = { ...recommendationUpdates, [recommendation.id]: updatedRecommendation };
		await invalidateSocial();
		if (accept) openRecommendationRequest(updatedRecommendation);
	}

	function openRecommendationRequest(recommendation: SocialRecommendation): void {
		if (recommendation.state !== 'ACCEPTED' || recommendation.requestId) return;
		pendingRequestRecommendation = recommendation;
		requestDestinationShelfId = recommendation.destinationShelfId ?? '';
		requestDestinationDeviceId = '';
		actionError = null;
	}

	function cancelRecommendationRequest(): void {
		pendingRequestRecommendation = null;
		requestDestinationShelfId = '';
		requestDestinationDeviceId = '';
	}

	async function submitRecommendationRequest(): Promise<void> {
		const recommendation = pendingRequestRecommendation;
		if (!recommendation || requestedRecommendationIds.has(recommendation.id)) return;
		const destination: Record<string, string> = {};
		if (requestDestinationShelfId) destination.shelfId = requestDestinationShelfId;
		if (requestDestinationDeviceId) destination.deviceId = requestDestinationDeviceId;
		const result = await runAction(`recommendation-request:${recommendation.id}`, () =>
			socialRequest(RequestRecommendationDocument, {
				id: recommendation.id,
				destination: Object.keys(destination).length > 0 ? destination : null
			})
		);
		if (!result) return;
		const updatedRecommendation = result.requestRecommendation;
		recommendationUpdates = { ...recommendationUpdates, [recommendation.id]: updatedRecommendation };
		if (updatedRecommendation.requestId || updatedRecommendation.handoffState === 'REQUESTED' || updatedRecommendation.handoffState === 'LINKED') {
			requestedRecommendationIds = new Set([...requestedRecommendationIds, recommendation.id]);
		}
		cancelRecommendationRequest();
		await invalidateSocial();
	}

	function requestRecommendation(recommendation: SocialRecommendation): void {
		openRecommendationRequest(recommendation);
	}

	async function revokeRecommendation(recommendation: SocialRecommendation): Promise<void> {
		const result = await runAction(`recommendation-revoke:${recommendation.id}`, () =>
			socialRequest(RevokeRecommendationDocument, { id: recommendation.id })
		);
		if (result) await invalidateSocial();
	}

	async function dismissRecommendation(recommendation: SocialRecommendation): Promise<void> {
		const result = await runAction(`recommendation-dismiss:${recommendation.id}`, () =>
			socialRequest(DismissRecommendationDocument, { id: recommendation.id })
		);
		if (result) await invalidateSocial();
	}

	async function createShareGrant(): Promise<void> {
		if (!selectedRecipient) {
			actionError = 'Choose a recipient from the active-user search before creating a share.';
			return;
		}
		if (!shareTargetKey.trim() || !shareTitle.trim() || !shareAuthors.trim()) {
			actionError = 'Add the visible book key, title, and authors before creating a share.';
			return;
		}
		if (shareScopes.size === 0) {
			actionError = 'Choose at least one field to share. Notes and highlights are opt-in.';
			return;
		}
		const input: Record<string, unknown> = {
			recipientUserId: selectedRecipient.id,
			targetKey: shareTargetKey.trim(),
			title: shareTitle.trim(),
			authors: shareAuthors.trim(),
			scopes: [...shareScopes]
		};
		if (shareProvider.trim()) input.sourceProvider = shareProvider.trim();
		if (shareRemoteId.trim()) input.remoteId = shareRemoteId.trim();
		if (shareExternalKey.trim()) input.externalKey = shareExternalKey.trim();
		if (shareExpiresAt) input.expiresAt = new Date(shareExpiresAt).toISOString();
		const result = await runAction('create-share-grant', () =>
			socialRequest(CreateShareGrantDocument, { input })
		);
		if (!result) return;
		localSentShares = [...localSentShares, result.createShareGrant];
		shareTargetKey = '';
		shareTitle = '';
		shareAuthors = '';
		shareProvider = '';
		shareRemoteId = '';
		shareExternalKey = '';
		shareExpiresAt = '';
		shareScopes = new Set(['METADATA']);
		await invalidateSocial();
	}

	async function respondToShareGrant(grant: SocialShareGrant, accept: boolean): Promise<void> {
		const result = await runAction(`share-response:${grant.id}`, () =>
			socialRequest(RespondToShareGrantDocument, { id: grant.id, accept })
		);
		if (result) await invalidateSocial();
	}

	async function revokeShareGrant(grant: SocialShareGrant): Promise<void> {
		const result = await runAction(`share-revoke:${grant.id}`, () =>
			socialRequest(RevokeShareGrantDocument, { id: grant.id })
		);
		if (result) await invalidateSocial();
	}

	async function toggleOverlay(overlay: SocialShareOverlay): Promise<void> {
		const result = await runAction(`overlay:${overlay.id}`, () =>
			socialRequest(SetOverlayVisibilityDocument, { overlayId: overlay.id, hidden: !overlay.hidden })
		);
		if (result) await invalidateSocial();
	}

	async function inviteMember(clubId: string, userId: string, role: string): Promise<void> {
		const result = await runAction(`club-invite:${clubId}:${userId}`, () =>
			socialRequest(CreateBookClubInvitationDocument, { id: clubId, input: { userId, role } })
		);
		if (result) await invalidateSocial();
	}

	async function respondToInvitation(invitation: BookClubInvitation, accept: boolean): Promise<void> {
		const result = await runAction(`club-invitation:${invitation.id}`, () =>
			socialRequest(RespondToBookClubInvitationDocument, {
				id: invitation.id,
				input: { accept }
			})
		);
		if (result) await invalidateSocial();
	}

	async function removeMember(clubId: string, memberId: string): Promise<void> {
		const result = await runAction(`club-remove:${memberId}`, () =>
			socialRequest(RemoveBookClubMemberDocument, { bookClubId: clubId, memberId })
		);
		if (result) await invalidateSocial();
	}

	async function leaveClub(clubId: string): Promise<void> {
		const result = await runAction(`club-leave:${clubId}`, () =>
			socialRequest(LeaveBookClubDocument, { bookClubId: clubId })
		);
		if (result) await invalidateSocial();
	}

	async function createClub(input: {
		name: string;
		description: string;
		isPrivate: boolean;
		creatorHideProgress: boolean;
	}): Promise<void> {
		const result = await runAction('club-create', () =>
			socialRequest(CreateBookClubDocument, { input })
		);
		if (result) await invalidateSocial();
	}

	function adaptiveReason(recommendation: AdaptiveRecommendation): string {
		if (recommendation.reasonCode === 'SIMILAR_TO_READING') return 'Similar to books you have enjoyed';
		if (recommendation.reasonCode === 'UNFINISHED_SERIES') return 'Continues a series you started';
		if (recommendation.reasonCode === 'ADAPTIVE_MATCH') return 'Matches your recent reading pattern';
		return 'Selected from your reading preferences';
	}

</script>

<svelte:head>
	<title>Social reading · Coppice</title>
	<meta name="description" content="Private recommendations, BookClub invitations, and explicit read-only sharing controls." />
</svelte:head>

<div class="flex flex-col gap-8">
	<PageHeader
		eyebrow="Private reading"
		title="Social reading"
		description="Choose who receives a recommendation or sharing grant. Acceptance never implies access to progress or annotations."
	/>

	{#if actionError}
		<Alert variant="destructive">
			<AlertTitle>Action could not be completed</AlertTitle>
			<AlertDescription class="flex items-start justify-between gap-3">
				<span>{actionError}</span>
				<Button type="button" variant="ghost" size="sm" aria-label="Dismiss error" onclick={() => (actionError = null)}><XIcon class="size-4" /></Button>
			</AlertDescription>
		</Alert>
	{/if}
	{#if pendingRequestRecommendation}
		<section aria-labelledby="recommendation-request-heading">
			<Card class="border-primary/50">
				<CardHeader>
					<CardTitle id="recommendation-request-heading" class="text-base">
						{pendingRequestRecommendation.targetKind === 'EXTERNAL_WORK' ? 'Request this book' : 'Queue this title'}
					</CardTitle>
					<CardDescription>
						{pendingRequestRecommendation.title} · {formatAuthors(pendingRequestRecommendation.authors) || 'Author not provided'}
					</CardDescription>
				</CardHeader>
				<CardContent class="flex flex-col gap-4">
					<p class="text-sm text-muted-foreground">
						Choose an optional shelf or device destination. The request service handles approval and private source handoff; tracker credentials and raw URLs never enter Coppice.
					</p>
					{#if destinationsQuery.isPending}
						<Skeleton class="h-20 rounded-md" />
					{:else if destinationsQuery.isError}
						<Alert variant="destructive">
							<AlertTitle>Destinations unavailable</AlertTitle>
							<AlertDescription>{errorMessage(destinationsQuery.error, 'Could not load your shelves and devices.')}</AlertDescription>
						</Alert>
					{:else}
						<div class="grid gap-4 @md/page:grid-cols-2">
							<div>
								<label class="text-sm font-medium" for="recommendation-destination-shelf">Shelf (optional)</label>
								<select id="recommendation-destination-shelf" class="mt-1 h-9 w-full rounded-md border bg-background px-3 text-sm" bind:value={requestDestinationShelfId}>
									<option value="">No shelf</option>
									{#each requestShelves as shelf (shelf.id)}
										<option value={shelf.id}>{shelf.name}</option>
									{/each}
								</select>
							</div>
							<div>
								<label class="text-sm font-medium" for="recommendation-destination-device">Device (optional)</label>
								<select id="recommendation-destination-device" class="mt-1 h-9 w-full rounded-md border bg-background px-3 text-sm" bind:value={requestDestinationDeviceId}>
									<option value="">No device</option>
									{#each requestDevices as device (device.id)}
										<option value={device.id}>{device.name}</option>
									{/each}
								</select>
							</div>
						</div>
					{/if}
				</CardContent>
				<CardFooter class="flex flex-wrap gap-2 border-t pt-4">
					<Button type="button" disabled={busyAction === `recommendation-request:${pendingRequestRecommendation.id}` || destinationsQuery.isPending} onclick={submitRecommendationRequest}>
						<CheckIcon class="mr-1 size-4" />
						Confirm request
					</Button>
					<Button type="button" variant="ghost" disabled={busyAction === `recommendation-request:${pendingRequestRecommendation.id}`} onclick={cancelRecommendationRequest}>Cancel</Button>
				</CardFooter>
			</Card>
		</section>
	{/if}

	<section aria-labelledby="adaptive-recommendations-heading" class="flex flex-col gap-4">
		<div class="flex flex-wrap items-end justify-between gap-3">
			<div>
				<h2 id="adaptive-recommendations-heading" class="text-xl font-semibold tracking-tight">Recommended for you</h2>
				<p class="text-sm text-muted-foreground">Explanations are based on your own reading signals. Other readers are never named.</p>
			</div>
			<div class="flex items-center gap-3 rounded-md border px-3 py-2">
				<LightbulbIcon class="size-4 text-muted-foreground" aria-hidden="true" />
				<label class="flex items-center gap-2 text-sm" for="recommendations-opt-out">
					<input id="recommendations-opt-out" type="checkbox" checked={recommendationOptOut} disabled={busyAction === 'recommendation-opt-out'} onclick={toggleRecommendationOptOut} />
					<span>Opt out of adaptive recommendations</span>
				</label>
			</div>
		</div>
		{#if adaptiveQuery.isPending}
			<div class="grid gap-4 @sm/page:grid-cols-2 @2xl/page:grid-cols-3">
				{#each { length: 3 } as _, index (index)}<Skeleton class="h-36 rounded-xl" />{/each}
			</div>
		{:else if adaptiveQuery.isError}
			<Alert variant="destructive">
				<AlertTitle>Unable to load recommendations</AlertTitle>
				<AlertDescription>{errorMessage(adaptiveQuery.error, 'The recommendation service returned an error.')}</AlertDescription>
			</Alert>
		{:else if recommendationOptOut}
			<Empty class="rounded-xl border border-dashed">
				<EmptyHeader><EmptyTitle>Adaptive recommendations are off</EmptyTitle><EmptyDescription>Turn them back on whenever you want a private reading-based feed.</EmptyDescription></EmptyHeader>
			</Empty>
		{:else if visibleAdaptiveRecommendations.length === 0}
			<Empty class="rounded-xl border border-dashed">
				<EmptyHeader><EmptyTitle>No new recommendations</EmptyTitle><EmptyDescription>Keep reading or rate a title to give the local recommender more signal.</EmptyDescription></EmptyHeader>
			</Empty>
		{:else}
			<div class="grid gap-4 @sm/page:grid-cols-2 @2xl/page:grid-cols-3">
				{#each visibleAdaptiveRecommendations as recommendation (recommendation.targetKey)}
					<Card>
						<CardHeader class="flex-row items-start gap-3 space-y-0">
							<Cover src={recommendation.coverUrl} alt="Cover for {recommendation.title}" aspect="square" class="size-14 rounded-md border" />
							<div class="min-w-0"><CardTitle class="text-base">{recommendation.title}</CardTitle><p class="mt-1 text-sm text-muted-foreground">{formatAuthors(recommendation.authors) || 'Author not provided'}</p></div>
						</CardHeader>
						<CardContent><p class="text-sm text-muted-foreground">{adaptiveReason(recommendation)}</p></CardContent>
						<CardFooter class="flex flex-wrap gap-2 border-t pt-4">
							<Button type="button" size="sm" onclick={() => prepareAdaptiveRecommendation(recommendation)}><SendIcon class="mr-1 size-4" />Recommend to someone</Button>
							<Button type="button" size="sm" variant="ghost" onclick={() => dismissAdaptiveRecommendation(recommendation)}>Dismiss</Button>
						</CardFooter>
					</Card>
				{/each}
			</div>
		{/if}
	</section>

	<section id="recommendation-form" aria-labelledby="recommendation-form-heading">
		<Card>
			<CardHeader>
			<CardTitle id="recommendation-form-heading" class="flex items-center gap-2 text-base"><SendIcon class="size-4" />Send a recommendation</CardTitle>
			<CardDescription>Pick an active user and share only a title snapshot. Acceptance opens a request confirmation with your own shelf or device choice; tracker details never enter Coppice.</CardDescription>
		</CardHeader>
		<CardContent class="flex flex-col gap-4">
			<div class="grid gap-4 @md/page:grid-cols-2">
				<div class="relative">
					<label class="text-sm font-medium" for="recommend-recipient">Recipient</label>
					{#if selectedRecipient}
						<div class="mt-1 flex items-center justify-between rounded-md border px-3 py-2 text-sm">
							<span>@{selectedRecipient.username}</span>
							<Button type="button" variant="ghost" size="sm" aria-label="Change recipient" onclick={clearRecipient}><XIcon class="size-4" /></Button>
						</div>
					{:else}
						<Input id="recommend-recipient" class="mt-1" bind:value={recipientSearch} placeholder="Search username (2+ characters)" autocomplete="off" />
						{#if recipientsQuery.isFetching}
							<p class="mt-1 text-xs text-muted-foreground">Searching active users…</p>
						{:else if recipientsQuery.isError}
							<p class="mt-1 text-sm text-destructive">{errorMessage(recipientsQuery.error, 'User search failed.')}</p>
						{:else if recipientSearch.trim().length >= 2 && (recipientsQuery.data?.socialUserSearch?.length ?? 0) > 0}
							<ul class="absolute z-10 mt-1 max-h-48 w-full overflow-auto rounded-md border bg-popover p-1 shadow-md" aria-label="Active users">
								{#each recipientsQuery.data?.socialUserSearch ?? [] as recipient (recipient.id)}
									<li><button type="button" class="flex w-full rounded-sm px-2 py-2 text-left text-sm hover:bg-accent" onclick={() => selectRecipient(recipient)}>@{recipient.username}</button></li>
								{/each}
							</ul>
						{:else if recipientSearch.trim().length >= 2}
							<p class="mt-1 text-sm text-muted-foreground">No active user matched that search.</p>
						{/if}
					{/if}
				</div>
				<div>
					<label class="text-sm font-medium" for="recommend-target-kind">Title source</label>
					<select id="recommend-target-kind" class="mt-1 h-9 w-full rounded-md border bg-background px-3 text-sm" bind:value={targetKind} onchange={clearRecommendationTarget}>
						<option value="EXTERNAL_WORK">External title (not in library)</option>
						<option value="INTERNAL_MEDIA">Library media</option>
						<option value="INTERNAL_WORK">Library work</option>
					</select>
				</div>
			</div>
			{#if targetKind !== 'EXTERNAL_WORK'}
				<div><label class="text-sm font-medium" for="recommend-target-id">Visible media/work ID</label><Input id="recommend-target-id" class="mt-1" bind:value={targetId} placeholder="Paste the library ID" autocomplete="off" /></div>
			{/if}
			<div class="grid gap-4 @md/page:grid-cols-2">
				<div><label class="text-sm font-medium" for="recommend-title">Title</label><Input id="recommend-title" class="mt-1" bind:value={recommendationTitle} placeholder="Book title" /></div>
				<div><label class="text-sm font-medium" for="recommend-authors">Authors</label><Input id="recommend-authors" class="mt-1" bind:value={recommendationAuthors} placeholder="Author names" /></div>
				<div><label class="text-sm font-medium" for="recommend-provider">Source provider <span class="font-normal text-muted-foreground">(optional)</span></label><Input id="recommend-provider" class="mt-1" bind:value={recommendationProvider} placeholder="open-library" /></div>
				<div><label class="text-sm font-medium" for="recommend-external-key">External key <span class="font-normal text-muted-foreground">(optional)</span></label><Input id="recommend-external-key" class="mt-1" bind:value={recommendationExternalKey} placeholder="Provider work key" autocomplete="off" /></div>
			</div>
			<div class="grid gap-4 @md/page:grid-cols-2">
				<div><label class="text-sm font-medium" for="recommend-remote-id">Remote ID <span class="font-normal text-muted-foreground">(optional)</span></label><Input id="recommend-remote-id" class="mt-1" bind:value={recommendationRemoteId} placeholder="Provider ID" autocomplete="off" /></div>
				<div><label class="text-sm font-medium" for="recommend-cover-url">Cover URL <span class="font-normal text-muted-foreground">(optional)</span></label><Input id="recommend-cover-url" class="mt-1" bind:value={recommendationCoverUrl} placeholder="https://…" inputmode="url" /></div>
			</div>
			<div><label class="text-sm font-medium" for="recommend-message">Note <span class="font-normal text-muted-foreground">(optional)</span></label><textarea id="recommend-message" class="mt-1 min-h-20 w-full rounded-md border bg-background px-3 py-2 text-sm" bind:value={recommendationMessage} maxlength="500" placeholder="Why might they enjoy it?"></textarea></div>
		</CardContent>
		<CardFooter class="border-t pt-4"><Button type="button" disabled={busyAction === 'send-recommendation' || !selectedRecipient} onclick={sendRecommendation}><SendIcon class="mr-1 size-4" />Send recommendation</Button></CardFooter>
	</Card>
	</section>

	<section aria-labelledby="incoming-recommendations-heading" class="flex flex-col gap-4">
		<div><h2 id="incoming-recommendations-heading" class="text-xl font-semibold tracking-tight">Recommendations for you</h2><p class="text-sm text-muted-foreground">Accept or decline explicitly. Accepted titles can continue into a request with your own shelf or device choice.</p></div>
		{#if incomingRecommendationsQuery.isPending}
			<div class="grid gap-4 @sm/page:grid-cols-2 @2xl/page:grid-cols-3">{#each { length: 3 } as _, index (index)}<Skeleton class="h-64 rounded-xl" />{/each}</div>
		{:else if incomingRecommendationsQuery.isError}
			<Alert variant="destructive"><AlertTitle>Unable to load incoming recommendations</AlertTitle><AlertDescription>{errorMessage(incomingRecommendationsQuery.error, 'The server returned an error.')}</AlertDescription></Alert>
		{:else if incomingRecommendations.length === 0}
			<Empty class="rounded-xl border border-dashed"><EmptyHeader><EmptyTitle>No incoming recommendations</EmptyTitle><EmptyDescription>Recommendations shared with you will appear here with an explanation and explicit actions.</EmptyDescription></EmptyHeader></Empty>
		{:else}
			<div class="grid gap-4 @sm/page:grid-cols-2 @2xl/page:grid-cols-3">
				{#each incomingRecommendations as recommendation (recommendation.id)}
					<RecommendationCard recommendation={recommendation} direction="incoming" adaptive={adaptiveRecommendations} requestSubmitted={requestedRecommendationIds.has(recommendation.id)} busy={busyAction === `recommendation-response:${recommendation.id}` || busyAction === `recommendation-dismiss:${recommendation.id}` || busyAction === `recommendation-request:${recommendation.id}`} onAccept={(value) => respondToRecommendation(value, true)} onDecline={(value) => respondToRecommendation(value, false)} onDismiss={dismissRecommendation} onRequest={requestRecommendation} />
				{/each}
			</div>
		{/if}
	</section>

	<section aria-labelledby="sent-recommendations-heading" class="flex flex-col gap-4">
		<div><h2 id="sent-recommendations-heading" class="text-xl font-semibold tracking-tight">Sent recommendations</h2><p class="text-sm text-muted-foreground">Revoke a pending recommendation before it is accepted. The recipient sees only the title snapshot.</p></div>
		{#if outgoingRecommendationsQuery.isPending}
			<Skeleton class="h-36 rounded-xl" />
		{:else if outgoingRecommendationsQuery.isError}
			<Alert variant="destructive"><AlertTitle>Unable to load sent recommendations</AlertTitle><AlertDescription>{errorMessage(outgoingRecommendationsQuery.error, 'The server returned an error.')}</AlertDescription></Alert>
		{:else if sentRecommendations.length === 0}
			<Empty class="rounded-xl border border-dashed"><EmptyHeader><EmptyTitle>Nothing sent yet</EmptyTitle><EmptyDescription>Use the form above to recommend a title to one active user.</EmptyDescription></EmptyHeader></Empty>
		{:else}
			<div class="grid gap-4 @sm/page:grid-cols-2 @2xl/page:grid-cols-3">
				{#each sentRecommendations as recommendation (recommendation.id)}
					<RecommendationCard recommendation={recommendation} direction="outgoing" busy={busyAction === `recommendation-revoke:${recommendation.id}`} onRevoke={revokeRecommendation} />
				{/each}
			</div>
		{/if}
	</section>

	<section aria-labelledby="sharing-heading" class="flex flex-col gap-4">
		<div><h2 id="sharing-heading" class="text-xl font-semibold tracking-tight">Scoped sharing</h2><p class="text-sm text-muted-foreground">A recommendation is not consent to share reading data. Each field below requires a separate grant, and annotations are always read-only snapshots.</p></div>
		<Card>
			<CardHeader>
			<CardTitle class="flex items-center gap-2 text-base"><Share2Icon class="size-4" />Create a share grant</CardTitle>
			<CardDescription>Choose the recipient above, then choose exactly what they may see. Notes and highlights are off by default.</CardDescription>
		</CardHeader>
		<CardContent class="flex flex-col gap-4">
			{#if sharingOptOut}
				<Alert><AlertTitle>Sharing is disabled for this account</AlertTitle><AlertDescription>A server policy currently prevents new grants. Existing grants remain visible with their current status.</AlertDescription></Alert>
			{/if}
			<div class="grid gap-4 @md/page:grid-cols-2">
				<div><label class="text-sm font-medium" for="share-target-key">Visible book key</label><Input id="share-target-key" class="mt-1" bind:value={shareTargetKey} placeholder="Library media/work key" autocomplete="off" /></div>
				<div><label class="text-sm font-medium" for="share-title">Title</label><Input id="share-title" class="mt-1" bind:value={shareTitle} placeholder="Book title" /></div>
				<div><label class="text-sm font-medium" for="share-authors">Authors</label><Input id="share-authors" class="mt-1" bind:value={shareAuthors} placeholder="Author names" /></div>
				<div><label class="text-sm font-medium" for="share-provider">Source provider <span class="font-normal text-muted-foreground">(optional)</span></label><Input id="share-provider" class="mt-1" bind:value={shareProvider} placeholder="open-library" /></div>
				<div><label class="text-sm font-medium" for="share-expires">Expires <span class="font-normal text-muted-foreground">(optional)</span></label><Input id="share-expires" class="mt-1" type="datetime-local" bind:value={shareExpiresAt} /></div>
			</div>
			<div class="grid gap-2 @md/page:grid-cols-2 @2xl/page:grid-cols-3">
				{#each SOCIAL_SCOPES as scope (scope.key)}
					<label class="flex cursor-pointer items-start gap-3 rounded-md border p-3 has-[:checked]:border-primary has-[:checked]:bg-accent">
						<input type="checkbox" checked={shareScopes.has(scope.key)} onchange={() => toggleScope(scope.key)} />
						<span><span class="block text-sm font-medium">{scope.label}</span><span class="mt-1 block text-xs text-muted-foreground">{scope.description}</span></span>
					</label>
				{/each}
			</div>
			<p class="text-xs text-muted-foreground">No scope grants a locator, exact page, elapsed session time, source device, or raw Liseur payload.</p>
		</CardContent>
		<CardFooter class="border-t pt-4"><Button type="button" disabled={sharingOptOut || busyAction === 'create-share-grant' || !selectedRecipient} onclick={createShareGrant}><Share2Icon class="mr-1 size-4" />Create share request</Button></CardFooter>
		</Card>

		<div class="grid gap-4 @2xl/page:grid-cols-2">
			<div class="flex flex-col gap-3">
				<h3 class="text-base font-semibold">Incoming share requests</h3>
				{#if incomingSharesQuery.isPending}<Skeleton class="h-48 rounded-xl" />{:else if incomingSharesQuery.isError}<Alert variant="destructive"><AlertTitle>Unable to load incoming shares</AlertTitle><AlertDescription>{errorMessage(incomingSharesQuery.error, 'The server returned an error.')}</AlertDescription></Alert>{:else if incomingShares.length === 0}<p class="rounded-md border border-dashed p-4 text-sm text-muted-foreground">No incoming share requests.</p>{:else}{#each incomingShares as grant (grant.id)}<ShareGrantCard grant={grant} direction="incoming" busy={busyAction === `share-response:${grant.id}`} onAccept={(value) => respondToShareGrant(value, true)} onDecline={(value) => respondToShareGrant(value, false)} />{/each}{/if}
			</div>
			<div class="flex flex-col gap-3">
				<h3 class="text-base font-semibold">Sent share grants</h3>
				{#if participantSharesQuery.isPending}<Skeleton class="h-48 rounded-xl" />{:else if participantSharesQuery.isError}<Alert variant="destructive"><AlertTitle>Unable to load sent shares</AlertTitle><AlertDescription>{errorMessage(participantSharesQuery.error, 'The server returned an error.')}</AlertDescription></Alert>{:else if sentShares.length === 0 && localSentShares.length === 0}<p class="rounded-md border border-dashed p-4 text-sm text-muted-foreground">No outgoing grants.</p>{:else}{#each [...sentShares, ...localSentShares.filter((local) => !sentShares.some((remote) => remote.id === local.id))] as grant (grant.id)}<ShareGrantCard grant={grant} direction="outgoing" busy={busyAction === `share-revoke:${grant.id}`} onRevoke={revokeShareGrant} />{/each}{/if}
			</div>
		</div>
	</section>

	<section aria-labelledby="shared-overlays-heading" class="flex flex-col gap-4">
		<div><h2 id="shared-overlays-heading" class="text-xl font-semibold tracking-tight">Shared highlights and notes</h2><p class="text-sm text-muted-foreground">Overlays are immutable projections. Hide one for yourself without changing the source reader's annotation.</p></div>
		<ul class="flex flex-wrap gap-3 text-xs text-muted-foreground" aria-label="Supported Liseur highlight colors">
			{#each Object.entries(LISEUR_COLORS) as [key, color] (key)}
				<li class="flex items-center gap-2">
					<span class="size-4 rounded-full border border-foreground/30" style:background-color={color.hex} role="img" aria-label="{color.label} highlight color"></span>
					<span>{color.label}</span>
				</li>
			{/each}
		</ul>
		{#if overlaysQuery.isPending}<div class="grid gap-4 @sm/page:grid-cols-2 @2xl/page:grid-cols-3">{#each { length: 3 } as _, index (index)}<Skeleton class="h-56 rounded-xl" />{/each}</div>{:else if overlaysQuery.isError}<Alert variant="destructive"><AlertTitle>Unable to load shared overlays</AlertTitle><AlertDescription>{errorMessage(overlaysQuery.error, 'The server returned an error.')}</AlertDescription></Alert>{:else if overlays.length === 0}<Empty class="rounded-xl border border-dashed"><EmptyHeader><EmptyTitle>No shared overlays</EmptyTitle><EmptyDescription>Accepted annotation grants will appear here. They never write into your private annotations.</EmptyDescription></EmptyHeader></Empty>{:else}<div class="grid gap-4 @sm/page:grid-cols-2 @2xl/page:grid-cols-3">{#each overlays as overlay (overlay.id)}<SharedOverlayCard overlay={overlay} busy={busyAction === `overlay:${overlay.id}`} onToggleVisibility={toggleOverlay} />{/each}</div>{/if}
	</section>

	<section aria-labelledby="bookclubs-heading" class="flex flex-col gap-4">
		<div><h2 id="bookclubs-heading" class="text-xl font-semibold tracking-tight">BookClubs</h2><p class="text-sm text-muted-foreground">Membership and invitations are explicit. They never substitute for a scoped sharing grant.</p></div>
		{#if clubsQuery.isPending || invitationsQuery.isPending}<Skeleton class="h-80 rounded-xl" />{:else if clubsQuery.isError || invitationsQuery.isError}<Alert variant="destructive"><AlertTitle>Unable to load BookClubs</AlertTitle><AlertDescription>{errorMessage(clubsQuery.error ?? invitationsQuery.error, 'The server returned an error.')}</AlertDescription></Alert>{:else}<BookClubPanel clubs={clubs as readonly BookClub[]} invitations={invitations as readonly BookClubInvitation[]} canCreate={canCreateBookClub} busy={busyAction?.startsWith('club-') ?? false} error={actionError} onInvite={inviteMember} onRemove={removeMember} onLeave={leaveClub} onRespondInvitation={respondToInvitation} onCreate={createClub} />{/if}
	</section>
</div>
