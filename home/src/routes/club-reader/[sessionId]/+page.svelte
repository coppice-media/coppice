<script lang="ts">
	import { onMount } from 'svelte';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@stump/ui/components/ui/card';
	import { Progress } from '@stump/ui/components/ui/progress';
	import AudioReader from '$lib/components/reader/AudioReader.svelte';
	import EpubReader from '$lib/components/reader/EpubReader.svelte';
	import PagedReader from '$lib/components/reader/PagedReader.svelte';
	import ClubReaderDiscussion from '$lib/components/social/ClubReaderDiscussion.svelte';
	import type { ReaderLocator } from '$lib/components/reader/locator';
	import type { ReaderResourceAdapter } from '$lib/components/reader/rwpm';
	import {
		ClubReaderApiError,
		clubReaderAudioUrl,
		clubReaderPageUrl,
		createClubReaderAnnotation,
		createClubReaderResources,
		deleteClubReaderAnnotation,
		getClubReaderSnapshot,
		redeemClubReaderSession,
		saveClubReaderProgress,
		subscribeClubReaderEvents,
		updateClubReaderAnnotation,
		updateClubReaderProfile,
		type ClubReaderChangeKind,
		type ClubReaderAnnotation,
		type ClubReaderAnnotationInput,
		type ClubReaderAnnotationKind,
		type ClubReaderAnnotationPatch,
		type ClubReaderProgressInput,
		type ClubReaderSnapshot
	} from '$lib/club-reader-api';
	import type { ReadiumLocatorInput } from '$lib/graphql/generated/graphql';
	import type { PageData } from './$types';

	let { data }: { data: PageData } = $props();
	const sessionId = $derived(data.sessionId);

	type ViewStatus = 'loading' | 'ready' | 'unavailable';
	type AnnotationDraft = {
		id: string | null;
		kind: ClubReaderAnnotationKind;
		locator: ReadiumLocatorInput | null;
		page: number | null;
		positionMs: number | null;
		excerpt: string;
		body: string;
		shared: boolean;
	};
	type PendingProgress = {
		participantId: string;
		bookId: string;
		generation: number;
		input: ClubReaderProgressInput;
	};
	type EpubReaderHandle = {
		getCurrentLocator(): ReadiumLocatorInput | null;
		jumpToAnnotation(locator: ReaderLocator): void;
	};
	type PagedReaderHandle = { jumpToPage(page: number): void };
	type AudioReaderHandle = {
		getCurrentPositionMs(): number;
		jumpToPosition(positionMs: number): void;
	};
	type DiscussionHandle = { refresh(): Promise<void> };
	const MAX_GUEST_ANNOTATION_CHARS = 10_000;
	const MAX_GUEST_LOCATOR_BYTES = 32 * 1024;
	const FALLBACK_POLL_MS = 60_000;
	const LIVE_MIN_BACKOFF_MS = 5_000;
	const LIVE_MAX_BACKOFF_MS = 60_000;

	let status = $state<ViewStatus>('loading');
	let statusMessage = $state('Opening your private reading link…');
	let snapshot = $state<ClubReaderSnapshot | null>(null);
	let actionError = $state<string | null>(null);
	let progressNotice = $state<string | null>(null);
	let profileBusy = $state(false);
	let annotationBusy = $state(false);
	let aliasDraft = $state('');
	let shareProgressDraft = $state(false);
	let annotationDraft = $state<AnnotationDraft | null>(null);
	let currentLocator = $state<ReadiumLocatorInput | null>(null);
	let currentProgression = $state(0);
	let currentPage = $state(1);
	let currentPositionMs = $state(0);
	let resourceAdapter = $state<ReaderResourceAdapter | null>(null);
	let epubReader = $state<EpubReaderHandle | null>(null);
	let pagedReader = $state<PagedReaderHandle | null>(null);
	let audioReader = $state<AudioReaderHandle | null>(null);
	let discussion = $state<DiscussionHandle | null>(null);
	let liveConnected = $state(false);

	let activeViewerId: string | null = null;
	let activeBookId: string | null = null;
	let activeBookGeneration = $state(0);
	let snapshotRevision = 0;
	let profileInitialized = false;
	let refreshInFlight = false;
	let pendingProgress: PendingProgress | null = null;
	let progressInFlight = false;
	let progressTimer: ReturnType<typeof setTimeout> | null = null;
	let refreshController: AbortController | null = null;
	let mounted = false;
	let bootstrapGeneration = 0;
	let openedSessionId: string | null = null;
	let closeLive: (() => void) | null = null;
	let liveGeneration = 0;
	let liveBackoffMs = LIVE_MIN_BACKOFF_MS;
	let liveRetryTimer: ReturnType<typeof setTimeout> | null = null;
	let liveRefreshTimer: ReturnType<typeof setTimeout> | null = null;
	let liveSnapshotWanted = false;
	let liveMessagesWanted = false;

	const book = $derived(snapshot?.book ?? null);
	const annotations = $derived(
		(snapshot?.annotations ?? []).filter(
			(annotation) =>
				annotation.bookId === book?.id && (annotation.shared || annotation.editable)
		)
	);
	const pageCount = $derived(
		book ? (book.visiblePages.length > 0 ? book.visiblePages.length : (book.pageCount ?? 0)) : 0
	);
	const guestPageSource = $derived.by(() => {
		const currentSessionId = sessionId;
		return (mediaId: string, page: number) =>
			clubReaderPageUrl(currentSessionId, mediaId, page);
	});
	const epubAnnotations = $derived(
		annotations.flatMap((annotation) =>
			annotation.kind === 'highlight' && annotation.locator
				? [{
						id: annotation.id,
						annotationText: annotation.excerpt ?? annotation.body ?? '',
						locator: annotation.locator
					}]
				: []
		)
	);
	const pagedAnnotations = $derived(
		annotations.map((annotation) => ({
			id: annotation.id,
			kind: annotation.kind,
			annotationText: annotation.excerpt ?? annotation.body ?? '',
			locator: annotation.locator,
			page: annotation.page
		}))
	);
	// Reader inputs are captured once per (viewer, book). Every progress save
	// returns a fresh snapshot; passing `snapshot`-derived objects straight to
	// the readers makes their open effects re-run (a prop getter tracks the
	// parent signal, not the primitive it returns), which reopened the EPUB
	// on every save and looped. `applySnapshot` refreshes this only on a
	// viewer or book change.
	type ReaderInit = {
		bookId: string;
		storedLocator: ReaderLocator | null;
		storedPercentage: number | undefined;
		startPage: number;
		startPositionMs: number;
		audio: (NonNullable<ClubReaderSnapshot['book']>['audio'] & object) | null;
	};
	let readerInit = $state.raw<ReaderInit | null>(null);
	const audioForReader = $derived(readerInit?.audio ?? null);

	function captureReaderInit(next: ClubReaderSnapshot): ReaderInit | null {
		const nextBook = next.book;
		if (!nextBook) return null;
		return {
			bookId: nextBook.id,
			storedLocator: next.progress?.locator ?? null,
			storedPercentage: next.progress?.progression,
			startPage: next.progress?.page ?? 1,
			startPositionMs: next.progress?.positionMs ?? 0,
			audio: nextBook.audio
				? {
						...nextBook.audio,
						tracks: nextBook.audio.tracks.map((track) => ({
							...track,
							url: clubReaderAudioUrl(next.session.id, nextBook.id, track.index)
						}))
					}
				: null
		};
	}

	function locatorInput(locator: ReaderLocator | null): ReadiumLocatorInput | null {
		if (!locator?.href) return null;
		return {
			chapterTitle: locator.chapterTitle,
			href: locator.href,
			title: locator.title,
			type: locator.type,
			locations: locator.locations
				? {
						fragments: locator.locations.fragments,
						progression: locator.locations.progression,
						position: locator.locations.position,
						totalProgression: locator.locations.totalProgression,
						cssSelector: locator.locations.cssSelector,
						partialCfi: locator.locations.partialCfi
					}
				: undefined,
			text: locator.text
				? {
						before: locator.text.before,
						highlight: locator.text.highlight,
						after: locator.text.after
					}
				: undefined
		};
	}

	function applySnapshot(next: ClubReaderSnapshot, syncProfile = false): void {
		const nextBookId = next.book?.id ?? null;
		const identityChanged = next.viewer.id !== activeViewerId;
		if (identityChanged) profileBusy = false;
		if (identityChanged || nextBookId !== activeBookId) {
			annotationBusy = false;
			activeViewerId = next.viewer.id;
			activeBookId = nextBookId;
			activeBookGeneration += 1;
			if (progressTimer !== null) clearTimeout(progressTimer);
			progressTimer = null;
			pendingProgress = null;
			annotationDraft = null;
			currentProgression = next.progress?.progression ?? 0;
			currentLocator = locatorInput(next.progress?.locator ?? null);
			currentPage = next.progress?.page ?? 1;
			currentPositionMs = next.progress?.positionMs ?? 0;
			resourceAdapter =
				next.book?.readerKind === 'epub'
					? createClubReaderResources(next.session.id, next.book.id)
					: null;
			readerInit = captureReaderInit(next);
		}
		snapshot = next;
		snapshotRevision += 1;
		if (syncProfile || identityChanged || !profileInitialized) {
			aliasDraft = next.viewer.displayName;
			shareProgressDraft = next.viewer.shareProgress;
			profileInitialized = true;
		}
		status = 'ready';
		statusMessage = '';
	}

	function publicFailure(cause: unknown): string {
		if (cause instanceof ClubReaderApiError) {
			if (cause.status === 401 || cause.status === 403 || cause.status === 404) {
				return 'This reading link is expired, revoked, or no longer available.';
			}
			if (cause.status === 409) {
				return 'The club’s active book changed. Refreshing the current reading assignment…';
			}
			return cause.message;
		}
		return cause instanceof Error ? cause.message : 'The book-club reader could not be opened.';
	}

	function loseAccess(cause: unknown): boolean {
		if (
			cause instanceof ClubReaderApiError &&
			(cause.status === 401 || cause.status === 403 || cause.status === 404)
		) {
			snapshot = null;
			status = 'unavailable';
			statusMessage = publicFailure(cause);
			if (progressTimer !== null) clearTimeout(progressTimer);
			progressTimer = null;
			pendingProgress = null;
			stopLive();
			return true;
		}
		return false;
	}

	/**
	 * `retry` means a live change could not be applied yet because a local edit
	 * owned the snapshot; a pending or in-flight progress save already returns a
	 * fresh server snapshot, so that case counts as settled.
	 */
	async function refreshSnapshot(): Promise<'settled' | 'retry'> {
		if (status !== 'ready') return 'settled';
		if (refreshInFlight) return 'retry';
		refreshInFlight = true;
		const revisionAtStart = snapshotRevision;
		refreshController?.abort();
		const controller = new AbortController();
		refreshController = controller;
		try {
			const next = await getClubReaderSnapshot(sessionId, controller.signal);
			if (controller.signal.aborted) return 'settled';
			if (progressInFlight || pendingProgress !== null || progressTimer !== null) return 'settled';
			if (snapshotRevision !== revisionAtStart || profileBusy || annotationBusy) return 'retry';
			applySnapshot(next);
		} catch (cause) {
			if (controller.signal.aborted) return 'settled';
			if (!loseAccess(cause) && !(cause instanceof ClubReaderApiError && cause.status === 409)) {
				actionError = publicFailure(cause);
			}
		} finally {
			if (refreshController === controller) {
				refreshController = null;
				refreshInFlight = false;
			}
		}
		return 'settled';
	}

	/** Coalesces live hints and retries until a refresh actually lands. */
	function scheduleLiveRefresh(delay: number): void {
		if (liveRefreshTimer !== null) clearTimeout(liveRefreshTimer);
		liveRefreshTimer = setTimeout(() => {
			liveRefreshTimer = null;
			void runLiveRefresh();
		}, delay);
	}

	async function runLiveRefresh(): Promise<void> {
		if (liveMessagesWanted) {
			liveMessagesWanted = false;
			void discussion?.refresh();
		}
		if (!liveSnapshotWanted) return;
		const generation = liveGeneration;
		liveSnapshotWanted = false;
		const outcome = await refreshSnapshot();
		if (outcome === 'retry' && generation === liveGeneration && status === 'ready') {
			liveSnapshotWanted = true;
			scheduleLiveRefresh(750);
		}
	}

	function onLiveChanged(kinds: ClubReaderChangeKind[]): void {
		if (kinds.some((kind) => kind !== 'messages')) liveSnapshotWanted = true;
		if (kinds.includes('messages')) liveMessagesWanted = true;
		scheduleLiveRefresh(250);
	}

	function stopLive(): void {
		liveGeneration += 1;
		closeLive?.();
		closeLive = null;
		liveConnected = false;
		if (liveRetryTimer !== null) clearTimeout(liveRetryTimer);
		liveRetryTimer = null;
		if (liveRefreshTimer !== null) clearTimeout(liveRefreshTimer);
		liveRefreshTimer = null;
		liveSnapshotWanted = false;
		liveMessagesWanted = false;
	}

	/**
	 * Opens the session's change stream. Events carry no content and are not
	 * replayed, so every (re)connect refetches the snapshot and the discussion.
	 * While the stream is down the 60 s fallback poll runs; when the browser gives
	 * up (401, 429, network) we re-check access and reconnect with backoff.
	 */
	function startLive(requestedSessionId: string): void {
		stopLive();
		if (typeof EventSource === 'undefined') return;
		const generation = liveGeneration;
		const isCurrent = () => generation === liveGeneration && requestedSessionId === sessionId;
		closeLive = subscribeClubReaderEvents(requestedSessionId, {
			onOpen: () => {
				if (!isCurrent()) return;
				liveConnected = true;
				liveBackoffMs = LIVE_MIN_BACKOFF_MS;
				onLiveChanged(['publication', 'messages']);
			},
			onChanged: (kinds) => {
				if (isCurrent()) onLiveChanged(kinds);
			},
			onRevoked: () => {
				if (!isCurrent()) return;
				loseAccess(new ClubReaderApiError(401, 'revoked'));
			},
			onDisconnected: (retrying) => {
				if (!isCurrent()) return;
				liveConnected = false;
				if (retrying) return;
				closeLive = null;
				void refreshSnapshot();
				const delay = liveBackoffMs;
				liveBackoffMs = Math.min(LIVE_MAX_BACKOFF_MS, liveBackoffMs * 2);
				liveRetryTimer = setTimeout(() => {
					liveRetryTimer = null;
					if (isCurrent() && mounted && status === 'ready') startLive(requestedSessionId);
				}, delay);
			}
		});
	}

	function beginSession(requestedSessionId: string): void {
		const generation = ++bootstrapGeneration;
		const incomingFragment = window.location.hash.slice(1);
		window.history.replaceState(
			window.history.state,
			'',
			`${window.location.pathname}${window.location.search}`
		);
		const token = new URLSearchParams(incomingFragment).get('token')?.trim() ?? '';

		snapshot = null;
		status = 'loading';
		statusMessage = 'Opening your private reading link…';
		actionError = null;
		progressNotice = null;
		aliasDraft = '';
		shareProgressDraft = false;
		profileInitialized = false;
		profileBusy = false;
		annotationBusy = false;
		annotationDraft = null;
		activeViewerId = null;
		activeBookId = null;
		activeBookGeneration += 1;
		snapshotRevision += 1;
		currentLocator = null;
		currentProgression = 0;
		currentPage = 1;
		currentPositionMs = 0;
		resourceAdapter = null;
		readerInit = null;
		if (progressTimer !== null) clearTimeout(progressTimer);
		progressTimer = null;
		pendingProgress = null;
		refreshController?.abort();
		refreshController = null;
		refreshInFlight = false;
		stopLive();

		const open = async () => {
			try {
				if (token) await redeemClubReaderSession(requestedSessionId, token);
				const initial = await getClubReaderSnapshot(requestedSessionId);
				if (!mounted || generation !== bootstrapGeneration || requestedSessionId !== sessionId) return;
				applySnapshot(initial, true);
				startLive(requestedSessionId);
			} catch (cause) {
				if (!mounted || generation !== bootstrapGeneration || requestedSessionId !== sessionId) return;
				status = 'unavailable';
				statusMessage = publicFailure(cause);
			}
		};
		void open();
	}

	$effect(() => {
		const requestedSessionId = sessionId;
		if (!mounted || requestedSessionId === openedSessionId) return;
		openedSessionId = requestedSessionId;
		beginSession(requestedSessionId);
	});

	onMount(() => {
		mounted = true;
		openedSessionId = sessionId;
		beginSession(sessionId);

		const onVisibilityChange = () => {
			if (document.visibilityState === 'visible') void refreshSnapshot();
		};
		document.addEventListener('visibilitychange', onVisibilityChange);
		// Opening another participant's link for this same session changes only
		// the fragment, so the page does not remount: redeem it now, before any
		// further write can go out under the previous reader's identity.
		const onHashChange = () => {
			if (new URLSearchParams(window.location.hash.slice(1)).has('token')) beginSession(sessionId);
		};
		window.addEventListener('hashchange', onHashChange);
		// The live stream replaces polling; this slow poll only covers a stream
		// that is down, reconnecting, or unsupported.
		const interval = setInterval(() => {
			if (document.visibilityState !== 'visible' || liveConnected) return;
			void refreshSnapshot();
			void discussion?.refresh();
		}, FALLBACK_POLL_MS);

		return () => {
			mounted = false;
			bootstrapGeneration += 1;
			clearInterval(interval);
			document.removeEventListener('visibilitychange', onVisibilityChange);
			window.removeEventListener('hashchange', onHashChange);
			refreshController?.abort();
			stopLive();
			if (progressTimer !== null) clearTimeout(progressTimer);
			progressTimer = null;
			if (pendingProgress) void flushProgress();
		};
	});

	function isCurrentReader(viewerId: string, bookId: string, generation: number): boolean {
		return (
			activeViewerId === viewerId &&
			activeBookId === bookId &&
			activeBookGeneration === generation &&
			snapshot?.viewer.id === viewerId &&
			snapshot.book?.id === bookId
		);
	}

	function queueProgress(
		participantId: string,
		bookId: string,
		generation: number,
		input: ClubReaderProgressInput
	): void {
		if (!isCurrentReader(participantId, bookId, generation) || !Number.isFinite(input.progression)) return;
		const safeInput = { ...input, progression: Math.min(1, Math.max(0, input.progression)) };
		currentProgression = safeInput.progression;
		pendingProgress = { participantId, bookId, generation, input: safeInput };
		if (progressTimer !== null) clearTimeout(progressTimer);
		progressTimer = setTimeout(() => {
			progressTimer = null;
			void flushProgress();
		}, 900);
	}

	async function flushProgress(): Promise<void> {
		if (progressTimer !== null) clearTimeout(progressTimer);
		progressTimer = null;
		if (progressInFlight) return;
		const pending = pendingProgress;
		pendingProgress = null;
		if (!pending || !isCurrentReader(pending.participantId, pending.bookId, pending.generation)) return;

		progressInFlight = true;
		const revisionAtStart = snapshotRevision;
		try {
			const updated = await saveClubReaderProgress(
				sessionId,
				pending.bookId,
				pending.participantId,
				pending.input
			);
			if (
				isCurrentReader(pending.participantId, pending.bookId, pending.generation) &&
				snapshotRevision === revisionAtStart
			) {
				applySnapshot(updated);
				progressNotice = updated.viewer.shareProgress
					? 'Your exact reading position is saved privately; guests see only coarse progress.'
					: 'Your private session progress is saved.';
			}
		} catch (cause) {
			if (isCurrentReader(pending.participantId, pending.bookId, pending.generation)) {
				if (!loseAccess(cause)) {
					if (cause instanceof ClubReaderApiError && cause.status === 409) {
						void refreshSnapshot();
					} else {
						actionError = publicFailure(cause);
					}
				}
			}
		} finally {
			progressInFlight = false;
			if (pendingProgress) {
				if (mounted) {
					progressTimer = setTimeout(() => {
						progressTimer = null;
						void flushProgress();
					}, 100);
				} else {
					// Unmounted mid-request: send the final position once, no timers.
					void flushProgress();
				}
			}
		}
	}

	function onEpubPosition(
		participantId: string,
		bookId: string,
		generation: number,
		payload: { locator: ReadiumLocatorInput; percentage: number; isComplete: boolean }
	): void {
		if (!isCurrentReader(participantId, bookId, generation)) return;
		currentLocator = payload.locator;
		queueProgress(participantId, bookId, generation, {
			progression: payload.percentage,
			locator: payload.locator,
			isComplete: payload.isComplete
		});
	}

	function onEpubSelection(
		participantId: string,
		bookId: string,
		generation: number,
		payload: { locator: ReadiumLocatorInput; excerpt: string }
	): void {
		if (
			!isCurrentReader(participantId, bookId, generation) ||
			book?.readerKind !== 'epub' ||
			!payload.excerpt.trim()
		) {
			return;
		}
		if (Array.from(payload.excerpt).length > MAX_GUEST_ANNOTATION_CHARS) {
			actionError = 'That selection is longer than the 10,000-character highlight limit. Select a shorter passage.';
			progressNotice = null;
			annotationDraft = null;
			return;
		}
		actionError = null;
		annotationDraft = {
			id: null,
			kind: 'highlight',
			locator: payload.locator,
			page: null,
			positionMs: null,
			excerpt: payload.excerpt,
			body: '',
			shared: false
		};
		progressNotice = 'Selected passage ready. It stays private unless you choose to share it.';
	}

	function onPage(participantId: string, bookId: string, generation: number, page: number): void {
		if (!isCurrentReader(participantId, bookId, generation)) return;
		currentPage = page;
		if (pageCount <= 0) return;
		queueProgress(participantId, bookId, generation, {
			progression: page / pageCount,
			page,
			isComplete: page >= pageCount
		});
	}

	function onAudioPosition(
		participantId: string,
		bookId: string,
		generation: number,
		payload: { positionMs: number; trackIndex: number; isComplete: boolean }
	): void {
		if (!isCurrentReader(participantId, bookId, generation)) return;
		currentPositionMs = payload.positionMs;
		const duration = book?.audio?.durationMs ?? 0;
		queueProgress(participantId, bookId, generation, {
			progression: duration > 0 ? payload.positionMs / duration : 0,
			positionMs: payload.positionMs,
			isComplete: payload.isComplete
		});
	}

	function startNote(): void {
		if (!book) return;
		annotationDraft = {
			id: null,
			kind: 'note',
			locator:
				book.readerKind === 'epub'
					? (epubReader?.getCurrentLocator() ?? currentLocator)
					: null,
			page: book.readerKind === 'paged' && pageCount > 0 ? currentPage : null,
			positionMs:
				book.readerKind === 'audio' && (book.audio?.tracks.length ?? 0) > 0
					? (audioReader?.getCurrentPositionMs() ?? currentPositionMs)
					: null,
			excerpt: '',
			body: '',
			shared: false
		};
	}

	function editAnnotation(annotation: ClubReaderAnnotation): void {
		annotationDraft = {
			id: annotation.id,
			kind: annotation.kind,
			locator: locatorInput(annotation.locator),
			page: annotation.page,
			positionMs: annotation.positionMs,
			excerpt: annotation.excerpt ?? '',
			body: annotation.body ?? '',
			shared: annotation.shared
		};
	}

	function updateLocalAnnotation(updated: ClubReaderAnnotation): void {
		if (!snapshot) return;
		snapshot = {
			...snapshot,
			annotations: snapshot.annotations.some((annotation) => annotation.id === updated.id)
				? snapshot.annotations.map((annotation) => (annotation.id === updated.id ? updated : annotation))
				: [...snapshot.annotations, updated]
		};
		snapshotRevision += 1;
	}

	async function saveAnnotation(event: SubmitEvent): Promise<void> {
		event.preventDefault();
		const current = snapshot;
		const draft = annotationDraft;
		if (!current?.book || !draft || annotationBusy) return;
		const requestedSessionId = sessionId;
		const participantId = current.viewer.id;
		const bookId = current.book.id;
		const generation = activeBookGeneration;
		const body = draft.body.trim();
		if (draft.kind === 'highlight' && !draft.excerpt.trim()) {
			actionError = 'Select real EPUB text before saving a highlight.';
			return;
		}
		if (draft.kind === 'note' && !body) {
			actionError = 'Write a note before saving it.';
			return;
		}
		if (Array.from(body).length > MAX_GUEST_ANNOTATION_CHARS) {
			actionError = 'Annotation notes are limited to 10,000 characters.';
			return;
		}
		if (
			!draft.id &&
			draft.kind === 'highlight' &&
			Array.from(draft.excerpt).length > MAX_GUEST_ANNOTATION_CHARS
		) {
			actionError = 'That selection is longer than the 10,000-character highlight limit. Select a shorter passage.';
			return;
		}
		if (!draft.id && draft.locator) {
			const locatorJson = JSON.stringify(draft.locator);
			if (new TextEncoder().encode(locatorJson).byteLength > MAX_GUEST_LOCATOR_BYTES) {
				actionError = 'This EPUB locator is too large to save. Select a shorter passage or make a book note.';
				return;
			}
		}

		annotationBusy = true;
		actionError = null;
		try {
			let saved: ClubReaderAnnotation;
			if (draft.id) {
				const patch: ClubReaderAnnotationPatch = { body, shared: draft.shared };
				saved = await updateClubReaderAnnotation(
					requestedSessionId,
					bookId,
					draft.id,
					participantId,
					patch
				);
			} else {
				const input: ClubReaderAnnotationInput = {
					kind: draft.kind,
					locator: draft.locator ?? undefined,
					page: draft.page ?? undefined,
					positionMs: draft.positionMs ?? undefined,
					excerpt: draft.kind === 'highlight' ? draft.excerpt : undefined,
					body: body || undefined,
					shared: draft.shared
				};
				saved = await createClubReaderAnnotation(requestedSessionId, bookId, participantId, input);
			}
			if (!isCurrentReader(participantId, bookId, generation)) return;
			updateLocalAnnotation(saved);
			annotationDraft = null;
			progressNotice = saved.shared
				? 'Saved and shared with current guests.'
				: 'Saved privately. Only you can see this annotation.';
		} catch (cause) {
			if (isCurrentReader(participantId, bookId, generation) && !loseAccess(cause)) {
				if (cause instanceof ClubReaderApiError && cause.status === 409) {
					void refreshSnapshot();
				} else {
					actionError = publicFailure(cause);
				}
			}
		} finally {
			if (requestedSessionId === sessionId && activeBookGeneration === generation) {
				annotationBusy = false;
			}
		}
	}

	async function removeAnnotation(annotation: ClubReaderAnnotation): Promise<void> {
		const current = snapshot;
		if (!current?.book || !annotation.editable || annotationBusy) return;
		if (!window.confirm('Delete this annotation from your guest reading history?')) return;
		const requestedSessionId = sessionId;
		const bookId = current.book.id;
		const participantId = current.viewer.id;
		const generation = activeBookGeneration;
		annotationBusy = true;
		actionError = null;
		try {
			await deleteClubReaderAnnotation(requestedSessionId, bookId, annotation.id, participantId);
			if (!isCurrentReader(participantId, bookId, generation) || !snapshot) return;
			snapshot = {
				...snapshot,
				annotations: snapshot.annotations.filter((item) => item.id !== annotation.id)
			};
			snapshotRevision += 1;
			if (annotationDraft?.id === annotation.id) annotationDraft = null;
		} catch (cause) {
			if (isCurrentReader(participantId, bookId, generation) && !loseAccess(cause)) {
				if (cause instanceof ClubReaderApiError && cause.status === 409) {
					void refreshSnapshot();
				} else {
					actionError = publicFailure(cause);
				}
			}
		} finally {
			if (requestedSessionId === sessionId && activeBookGeneration === generation) {
				annotationBusy = false;
			}
		}
	}

	function annotationHasAnchor(annotation: ClubReaderAnnotation): boolean {
		if (!book) return false;
		if (book.readerKind === 'epub') return annotation.locator !== null;
		if (book.readerKind === 'paged') return annotation.page !== null;
		return annotation.positionMs !== null;
	}

	function jumpToAnnotation(annotation: ClubReaderAnnotation): void {
		if (!book || annotation.bookId !== book.id) return;
		if (book.readerKind === 'epub' && annotation.locator) {
			epubReader?.jumpToAnnotation(annotation.locator);
		} else if (book.readerKind === 'paged' && annotation.page !== null) {
			pagedReader?.jumpToPage(annotation.page);
		} else if (book.readerKind === 'audio' && annotation.positionMs !== null) {
			audioReader?.jumpToPosition(annotation.positionMs);
		}
	}

	async function saveProfile(event: SubmitEvent): Promise<void> {
		event.preventDefault();
		if (!snapshot || profileBusy) return;
		const displayName = aliasDraft.trim();
		if (!displayName) {
			actionError = 'Choose a display name for this guest session.';
			return;
		}
		profileBusy = true;
		actionError = null;
		const requestedSessionId = sessionId;
		const participantId = snapshot.viewer.id;
		try {
			const updated = await updateClubReaderProfile(requestedSessionId, participantId, {
				displayName,
				shareProgress: shareProgressDraft
			});
			const stillCurrentParticipant =
				requestedSessionId === sessionId && snapshot?.viewer.id === participantId;
			if (stillCurrentParticipant) applySnapshot(updated, true);
			if (stillCurrentParticipant) {
				progressNotice = updated.viewer.shareProgress
					? 'Your alias and coarse progress-sharing choice are saved.'
					: 'Your alias is saved; your progress remains private.';
			}
		} catch (cause) {
			if (requestedSessionId === sessionId && snapshot?.viewer.id === participantId && !loseAccess(cause)) {
				if (cause instanceof ClubReaderApiError && cause.status === 409) {
					void refreshSnapshot();
				} else {
					actionError = publicFailure(cause);
				}
			}
		} finally {
			if (requestedSessionId === sessionId && snapshot?.viewer.id === participantId) {
				profileBusy = false;
			}
		}
	}

	function safeGroupPercentage(value: number | null): number | null {
		if (value === null || !Number.isFinite(value)) return null;
		return Math.min(100, Math.max(0, Math.round(value / 5) * 5));
	}

	function displayOwnProgress(value: number): string {
		return `${(Math.min(1, Math.max(0, value)) * 100).toFixed(2)}%`;
	}

	function formatPosition(positionMs: number): string {
		const totalSeconds = Math.floor(Math.max(0, positionMs) / 1000);
		const hours = Math.floor(totalSeconds / 3600);
		const minutes = Math.floor((totalSeconds % 3600) / 60);
		const seconds = totalSeconds % 60;
		return `${hours}:${String(minutes).padStart(2, '0')}:${String(seconds).padStart(2, '0')}`;
	}
</script>

<svelte:head>
	<title>Book-club reader · Coppice</title>
	<meta name="description" content="A private, session-scoped book-club reader." />
	<meta name="referrer" content="no-referrer" />
</svelte:head>

<main class="mx-auto flex w-full max-w-6xl flex-col gap-5 px-4 py-6 sm:px-6">
	{#if status === 'loading'}
		<Card>
			<CardContent class="py-10 text-center text-sm text-muted-foreground">{statusMessage}</CardContent>
		</Card>
	{:else if status === 'unavailable'}
		<Alert variant="destructive">
			<AlertTitle>Guest reading is unavailable</AlertTitle>
			<AlertDescription>{statusMessage}</AlertDescription>
		</Alert>
	{:else if snapshot}
		<header class="flex flex-wrap items-start justify-between gap-3">
			<div>
				<p class="text-sm text-muted-foreground">Book-club guest reading</p>
				<h1 class="text-2xl font-semibold tracking-tight">{snapshot.session.name}</h1>
				<p class="mt-1 text-sm text-muted-foreground">
					Reading as {snapshot.viewer.displayName}. This session is separate from account reading history.
				</p>
			</div>
			<div class="flex flex-wrap items-center gap-2">
				<Badge variant="secondary">Private guest session</Badge>
				<Badge
					variant="outline"
					role="status"
					title={liveConnected
						? 'Changes from other readers appear within a second.'
						: 'The live connection is down; the page checks for changes every minute and keeps reconnecting.'}
				>
					{liveConnected ? 'Live updates' : 'Updates every minute'}
				</Badge>
			</div>
		</header>

		{#if actionError}
			<Alert variant="destructive">
				<AlertTitle>Could not save that change</AlertTitle>
				<AlertDescription>{actionError}</AlertDescription>
			</Alert>
		{/if}
		{#if progressNotice}
			<p role="status" class="text-sm text-muted-foreground">{progressNotice}</p>
		{/if}

		<Card>
			<CardHeader>
				<CardTitle>Your session identity and privacy</CardTitle>
				<CardDescription>
					Choose the alias guests see. Native account names and reading history are never imported.
				</CardDescription>
			</CardHeader>
			<CardContent>
				<form class="flex flex-col gap-4" onsubmit={saveProfile}>
					<label class="flex flex-col gap-1 text-sm font-medium">
						Display name for this session
						<input
							class="h-9 rounded-md border border-input bg-background px-3 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
							bind:value={aliasDraft}
							maxlength="80"
							required
							autocomplete="off"
						/>
					</label>
					<label class="flex items-start gap-2 text-sm">
						<input class="mt-1" type="checkbox" bind:checked={shareProgressDraft} />
						<span>
							Share my reading progress with guests in this session. When enabled, only a coarse
							five-percent percentage and completion state are visible; exact position and reading
							timing stay private.
						</span>
					</label>
					<div class="flex flex-wrap items-center gap-3">
						<Button type="submit" disabled={profileBusy || !aliasDraft.trim()}>
							{profileBusy ? 'Saving…' : 'Save privacy choices'}
						</Button>
						<p class="text-xs text-muted-foreground">
							{#if shareProgressDraft}
								After saving, only your five-percent-rounded percentage and completion state will be
								visible to current guests. Exact position and reading timing stay private.
							{:else}
								Your progress stays private to you. Each note or highlight is also private unless you
								share it separately.
							{/if}
						</p>
					</div>
				</form>
			</CardContent>
		</Card>

		<section aria-labelledby="group-progress-heading">
			<Card>
				<CardHeader>
					<CardTitle id="group-progress-heading">Group progress</CardTitle>
					<CardDescription>
						Only chosen aliases and explicitly shared, five-percent-rounded progress appear here.
						Updates appear live as readers move through the book.
					</CardDescription>
				</CardHeader>
				<CardContent>
					{#if snapshot.participants.length}
						<ul class="flex flex-col gap-4">
							{#each snapshot.participants as participant (participant.id)}
								{@const mayShowProgress =
									participant.id !== snapshot.viewer.id || snapshot.viewer.shareProgress}
								{@const percentage = mayShowProgress
									? safeGroupPercentage(participant.percentage)
									: null}
								{@const isComplete = mayShowProgress && participant.isComplete === true}
								<li class="flex flex-col gap-1.5">
									<div class="flex flex-wrap items-center justify-between gap-2 text-sm">
										<span class="font-medium">{participant.displayName}</span>
										{#if percentage !== null}
											<span class="text-muted-foreground">
												{isComplete ? 'Complete · ' : 'About '}{percentage}%
											</span>
										{:else}
											<span class="text-muted-foreground">No shared progress</span>
										{/if}
									</div>
									{#if percentage !== null}
										<Progress value={percentage} aria-label={`${participant.displayName} progress`} />
									{/if}
								</li>
							{/each}
						</ul>
					{:else}
						<p class="text-sm text-muted-foreground">No active guest progress has been shared.</p>
					{/if}
				</CardContent>
			</Card>
		</section>

		{#if book}
			<Card>
				<CardHeader>
					<CardTitle>{book.title}</CardTitle>
					<CardDescription>
						{#if book.readerKind === 'epub'}
							EPUB · your exact reading position is private to this session.
						{:else if book.readerKind === 'paged'}
							Paged book or PDF · page notes use the current visible page.
						{:else}
							Audiobook · notes use the current publication time.
						{/if}
					</CardDescription>
				</CardHeader>
				<CardContent class="flex flex-col gap-4">
					<div class="flex flex-wrap items-center gap-3">
						<div class="min-w-48 flex-1">
							<Progress value={currentProgression * 100} aria-label="Your exact reading progress" />
						</div>
						<span class="text-sm tabular-nums text-muted-foreground">
							Your position · {displayOwnProgress(currentProgression)}
						</span>
						{#if snapshot.progress?.isComplete}
							<Badge variant="secondary">Complete</Badge>
						{/if}
					</div>

					{#key `${snapshot.viewer.id}:${book.id}`}
						{@const readerViewerId = snapshot.viewer.id}
						{@const readerGeneration = activeBookGeneration}
						{@const init = readerInit}
						{#if !init || init.bookId !== book.id}
							<p class="text-sm text-muted-foreground">Opening the book…</p>
						{:else if book.readerKind === 'epub'}
							<EpubReader
								bind:this={epubReader}
								mediaId={init.bookId}
								storedLocator={init.storedLocator}
								storedPercentage={init.storedPercentage}
								annotations={epubAnnotations}
								resourceAdapter={resourceAdapter ?? undefined}
								onLocator={(payload) =>
									onEpubPosition(readerViewerId, book.id, readerGeneration, payload)}
								onSelection={(payload) =>
									onEpubSelection(readerViewerId, book.id, readerGeneration, payload)}
							/>
						{:else if book.readerKind === 'paged' && pageCount > 0}
							<PagedReader
								bind:this={pagedReader}
								mediaId={init.bookId}
								pageCount={pageCount}
								startPage={init.startPage}
								annotations={pagedAnnotations}
								pageSource={guestPageSource}
								onPage={(page) => onPage(readerViewerId, book.id, readerGeneration, page)}
							/>
						{:else if book.readerKind === 'audio' && audioForReader && audioForReader.tracks.length > 0}
							<AudioReader
								bind:this={audioReader}
								audio={audioForReader}
								startPositionMs={init.startPositionMs}
								onPosition={(payload) =>
									onAudioPosition(readerViewerId, book.id, readerGeneration, payload)}
							/>
						{:else}
							<Alert>
								<AlertTitle>This reader is not available yet</AlertTitle>
								<AlertDescription>
									The current book has no supported readable pages or audio tracks. Ask the organizer to
									publish a supported copy.
								</AlertDescription>
							</Alert>
						{/if}
					{/key}
				</CardContent>
			</Card>

			<section aria-labelledby="annotations-heading">
				<Card>
					<CardHeader>
					<CardTitle id="annotations-heading">Notes and highlights</CardTitle>
					<CardDescription>
						Private annotations belong only to you. Share a selected EPUB passage or note explicitly
						to make it visible to this session’s guests. No annotation is shared by default.
					</CardDescription>
				</CardHeader>
				<CardContent class="flex flex-col gap-4">
					<div class="flex flex-wrap items-center gap-2">
						<Button variant="outline" onclick={startNote} disabled={annotationBusy}>
							Add a note
						</Button>
						{#if book.readerKind === 'epub'}
							<p class="text-xs text-muted-foreground">
								Select passage text to create a durable anchored highlight.
							</p>
						{:else if book.readerKind === 'paged' && pageCount > 0}
							<p class="text-xs text-muted-foreground">A note will be anchored to page {currentPage}.</p>
						{:else if book.readerKind === 'paged'}
							<p class="text-xs text-muted-foreground">
								No readable page is available; this will be a book-level note.
							</p>
						{:else if (book.audio?.tracks.length ?? 0) > 0}
							<p class="text-xs text-muted-foreground">
								A note will be anchored to the current audiobook position.
							</p>
						{:else}
							<p class="text-xs text-muted-foreground">
								No playable track is available; this will be a book-level note.
							</p>
						{/if}
					</div>

					{#if annotationDraft}
						<form class="rounded-lg border p-4" onsubmit={saveAnnotation}>
							<div class="flex flex-col gap-3">
								<h3 class="font-medium">
									{annotationDraft.id
									? 'Edit annotation'
									: annotationDraft.kind === 'highlight'
										? 'Save selected passage'
										: 'Write a note'}
								</h3>
								{#if annotationDraft.excerpt}
									<blockquote class="border-l-2 pl-3 text-sm text-muted-foreground">
										{annotationDraft.excerpt}
									</blockquote>
								{/if}
								<label class="flex flex-col gap-1 text-sm font-medium">
									{annotationDraft.kind === 'highlight' ? 'Note about this passage (optional)' : 'Note'}
									<textarea
										class="min-h-24 rounded-md border border-input bg-background p-3 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring/50"
										bind:value={annotationDraft.body}
										maxlength={MAX_GUEST_ANNOTATION_CHARS}
										required={annotationDraft.kind === 'note'}
										placeholder="Write a private note unless you choose to share it."
									></textarea>
								</label>
								<label class="flex items-start gap-2 text-sm">
									<input class="mt-1" type="checkbox" bind:checked={annotationDraft.shared} />
									<span>Share this {annotationDraft.kind} with guests in this reading session</span>
								</label>
								<div class="flex flex-wrap gap-2">
									<Button type="submit" disabled={annotationBusy}>
										{annotationBusy ? 'Saving…' : annotationDraft.id ? 'Save changes' : 'Save privately or share'}
									</Button>
									<Button
										variant="ghost"
										type="button"
										onclick={() => (annotationDraft = null)}
										disabled={annotationBusy}
									>
										Cancel
									</Button>
								</div>
							</div>
						</form>
					{/if}

					{#if annotations.length}
						<ul class="flex flex-col gap-3">
							{#each annotations as annotation (annotation.id)}
								<li class="rounded-lg border p-4">
									<div class="flex flex-wrap items-start justify-between gap-3">
										<div class="min-w-0 flex-1">
											<div class="flex flex-wrap items-center gap-2">
												<Badge variant="outline">{annotation.kind}</Badge>
												<span class="text-sm font-medium">{annotation.authorName}</span>
												<Badge variant={annotation.shared ? 'secondary' : 'outline'}>
													{annotation.shared ? 'Shared with guests' : 'Private to author'}
												</Badge>
											</div>
											{#if annotation.excerpt}
												<blockquote class="mt-2 border-l-2 pl-3 text-sm">
													{annotation.excerpt}
												</blockquote>
											{/if}
											{#if annotation.body}
												<p class="mt-2 whitespace-pre-wrap text-sm">{annotation.body}</p>
											{/if}
											<p class="mt-2 text-xs text-muted-foreground">
												{#if book.readerKind === 'paged' && annotation.page !== null}
													Page {annotation.page}
												{:else if book.readerKind === 'audio' && annotation.positionMs !== null}
													{formatPosition(annotation.positionMs)}
												{:else if book.readerKind === 'epub' && !annotation.locator}
													Book note · not tied to a passage
												{:else if book.readerKind === 'epub'}
													EPUB passage
												{:else}
													Book note · not tied to a page or position
												{/if}
											</p>
										</div>
										<div class="flex flex-wrap gap-2">
											{#if annotationHasAnchor(annotation)}
												<Button
													variant="outline"
													size="sm"
													onclick={() => jumpToAnnotation(annotation)}
												>
													Jump to location
												</Button>
											{/if}
											{#if annotation.editable}
												<Button
													variant="ghost"
													size="sm"
													onclick={() => editAnnotation(annotation)}
													disabled={annotationBusy}
												>
													Edit
												</Button>
												<Button
													variant="ghost"
													size="sm"
													onclick={() => void removeAnnotation(annotation)}
													disabled={annotationBusy}
												>
													Delete
												</Button>
											{/if}
										</div>
									</div>
								</li>
							{/each}
						</ul>
					{:else}
						<p class="text-sm text-muted-foreground">No guest annotations on this book yet.</p>
					{/if}
				</CardContent>
			</Card>
			</section>
		{:else}
			<Alert>
				<AlertTitle>No book is published to this session</AlertTitle>
				<AlertDescription>
					The reader is paused until an organizer publishes the current club book. A queue change may
					also require the organizer to publish the new active book.
				</AlertDescription>
			</Alert>
		{/if}

		{#key `${snapshot.session.id}:${snapshot.viewer.id}`}
			<ClubReaderDiscussion
				bind:this={discussion}
				sessionId={snapshot.session.id}
				participantId={snapshot.viewer.id}
				bookId={book?.id ?? null}
				onAccessLost={loseAccess}
				onBookChanged={() => void refreshSnapshot()}
			/>
		{/key}
	{:else}
		<Alert variant="destructive">
			<AlertTitle>Guest reading is unavailable</AlertTitle>
			<AlertDescription>This session could not be loaded. No account sign-in is used for guest links.</AlertDescription>
		</Alert>
	{/if}
</main>
