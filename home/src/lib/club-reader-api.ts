import type { AudioChapterSource, ReadiumLocatorInput } from '$lib/graphql/generated/graphql';
import type { ReaderLocator } from '$lib/components/reader/locator';
import type { Publication, ReaderResourceAdapter, RwpmManifest, RwpmPositions } from '$lib/components/reader/rwpm';

const API_BASE = '/api/v2/club-reader';

export type ClubReaderKind = 'epub' | 'paged' | 'audio';
export type ClubReaderAnnotationKind = 'highlight' | 'note';

export interface ClubReaderAudioTrack {
	index: number;
	mime: string;
	durationMs: number;
	startOffsetMs: number;
	byteSize: number;
	url: string;
}

export interface ClubReaderAudio {
	durationMs: number;
	codec: string;
	chapterSource: AudioChapterSource;
	tracks: ClubReaderAudioTrack[];
	chapters: Array<{
		index: number;
		title: string | null;
		startMs: number;
		endMs: number | null;
	}>;
}

export interface ClubReaderBook {
	/** BookClub queue-row id, never a native media id. */
	id: string;
	title: string;
	extension: string;
	readerKind: ClubReaderKind;
	pageCount: number | null;
	visiblePages: number[];
	audio: ClubReaderAudio | null;
}

export interface ClubReaderProgress {
	progression: number;
	locator: ReaderLocator | null;
	page: number | null;
	positionMs: number | null;
	isComplete: boolean;
}

export interface ClubReaderParticipantProgress {
	id: string;
	displayName: string;
	percentage: number | null;
	isComplete: boolean | null;
}

export interface ClubReaderAnnotation {
	id: string;
	authorId: string;
	authorName: string;
	bookId: string;
	kind: ClubReaderAnnotationKind;
	locator: ReaderLocator | null;
	page: number | null;
	positionMs: number | null;
	excerpt: string | null;
	body: string | null;
	color: string | null;
	shared: boolean;
	editable: boolean;
}

export interface ClubReaderSnapshot {
	session: {
		id: string;
		name: string;
		clubId: string;
		expiresAt: string;
	};
	book: ClubReaderBook | null;
	viewer: {
		id: string;
		displayName: string;
		shareProgress: boolean;
	};
	progress: ClubReaderProgress | null;
	participants: ClubReaderParticipantProgress[];
	annotations: ClubReaderAnnotation[];
}

export interface ClubReaderProgressInput {
	progression: number;
	locator?: ReadiumLocatorInput;
	page?: number;
	positionMs?: number;
	isComplete?: boolean;
}

export interface ClubReaderAnnotationInput {
	kind: ClubReaderAnnotationKind;
	locator?: ReadiumLocatorInput;
	page?: number;
	positionMs?: number;
	excerpt?: string;
	body?: string;
	color?: string;
	shared?: boolean;
}

export type ClubReaderAnnotationPatch = Partial<ClubReaderAnnotationInput>;

/** Snapshot facets a live `changed` event can name; the event never carries content. */
export type ClubReaderChangeKind = 'progress' | 'annotations' | 'participants' | 'publication' | 'messages';

const CHANGE_KINDS: readonly ClubReaderChangeKind[] = [
	'progress',
	'annotations',
	'participants',
	'publication',
	'messages'
];

export const CLUB_READER_MESSAGE_MAX_CHARS = 4000;

/** A session-discussion message; `authorId` is null once the author is revoked. */
export interface ClubReaderMessage {
	id: string;
	authorId: string | null;
	authorName: string;
	bookId: string | null;
	body: string;
	createdAt: string;
	editedAt: string | null;
	mine: boolean;
}

/** Newest-first page; pass the last message id as `before` for the next page. */
export interface ClubReaderMessagePage {
	messages: ClubReaderMessage[];
	hasMore: boolean;
}

export class ClubReaderApiError extends Error {
	readonly status: number;

	constructor(status: number, message: string) {
		super(message);
		this.name = 'ClubReaderApiError';
		this.status = status;
	}
}

function sessionPath(sessionId: string): string {
	return `${API_BASE}/sessions/${encodeURIComponent(sessionId)}`;
}

function bookPath(sessionId: string, bookId: string): string {
	return `${sessionPath(sessionId)}/books/${encodeURIComponent(bookId)}`;
}

function packageResourcePath(packagePath: string): string {
	const segments = packagePath.split('/');
	if (
		segments.length === 0 ||
		segments.some((segment) => !segment || segment === '.' || segment === '..' || segment.includes('\\') || segment.includes('\0'))
	) {
		throw new Error('The EPUB contains an invalid resource path.');
	}
	return segments.map((segment) => encodeURIComponent(segment)).join('/');
}

function responseError(status: number): ClubReaderApiError {
	if (status === 401 || status === 403 || status === 404) {
		return new ClubReaderApiError(status, 'This reading link is expired, revoked, or no longer available.');
	}
	if (status === 409) {
		return new ClubReaderApiError(status, 'The club’s active book changed. The reader is refreshing.');
	}
	if (status === 429) {
		return new ClubReaderApiError(status, 'Too many messages in a short time. Wait a moment, then try again.');
	}
	return new ClubReaderApiError(status, 'The book-club reader could not complete that request.');
}

async function requestJson<T>(path: string, init: RequestInit = {}): Promise<T> {
	let response: Response;
	try {
		response = await fetch(path, {
			...init,
			credentials: 'same-origin',
			cache: 'no-store',
			redirect: 'error',
			headers: {
				Accept: 'application/json',
				...(init.body ? { 'Content-Type': 'application/json' } : {}),
				...init.headers
			}
		});
	} catch (cause) {
		if (cause instanceof Error && cause.name === 'AbortError') throw cause;
		throw new ClubReaderApiError(0, 'The book-club reader service could not be reached.');
	}
	if (response.redirected || !response.ok) throw responseError(response.status || 502);
	const contentType = response.headers.get('content-type')?.toLowerCase() ?? '';
	if (!contentType.includes('json')) {
		throw new ClubReaderApiError(response.status, 'The book-club reader returned an unexpected response.');
	}
	return (await response.json()) as T;
}

async function requestNoContent(path: string, init: RequestInit): Promise<void> {
	let response: Response;
	try {
		response = await fetch(path, {
			...init,
			credentials: 'same-origin',
			cache: 'no-store',
			redirect: 'error',
			headers: {
				Accept: 'application/json',
				...(init.body ? { 'Content-Type': 'application/json' } : {}),
				...init.headers
			}
		});
	} catch (cause) {
		if (cause instanceof Error && cause.name === 'AbortError') throw cause;
		throw new ClubReaderApiError(0, 'The book-club reader service could not be reached.');
	}
	if (response.redirected || !response.ok) throw responseError(response.status || 502);
}

export async function redeemClubReaderSession(sessionId: string, token: string): Promise<void> {
	const result = await requestJson<{ sessionId: string }>(`${API_BASE}/redeem`, {
		method: 'POST',
		body: JSON.stringify({ sessionId, token })
	});
	if (result.sessionId !== sessionId) {
		throw new ClubReaderApiError(502, 'The reading link did not match this session.');
	}
}

export function getClubReaderSnapshot(sessionId: string, signal?: AbortSignal): Promise<ClubReaderSnapshot> {
	return requestJson<ClubReaderSnapshot>(sessionPath(sessionId), { signal });
}

export function updateClubReaderProfile(
	sessionId: string,
	participantId: string,
	input: { displayName?: string; shareProgress?: boolean }
): Promise<ClubReaderSnapshot> {
	return requestJson<ClubReaderSnapshot>(`${sessionPath(sessionId)}/me`, {
		method: 'PATCH',
		body: JSON.stringify(input),
		headers: { 'X-Club-Reader-Participant': participantId }
	});
}

export function saveClubReaderProgress(
	sessionId: string,
	bookId: string,
	participantId: string,
	input: ClubReaderProgressInput
): Promise<ClubReaderSnapshot> {
	return requestJson<ClubReaderSnapshot>(`${bookPath(sessionId, bookId)}/progress`, {
		method: 'PUT',
		body: JSON.stringify(input),
		headers: { 'X-Club-Reader-Participant': participantId }
	});
}

export function createClubReaderAnnotation(
	sessionId: string,
	bookId: string,
	participantId: string,
	input: ClubReaderAnnotationInput
): Promise<ClubReaderAnnotation> {
	return requestJson<ClubReaderAnnotation>(`${bookPath(sessionId, bookId)}/annotations`, {
		method: 'POST',
		body: JSON.stringify(input),
		headers: { 'X-Club-Reader-Participant': participantId }
	});
}

export function updateClubReaderAnnotation(
	sessionId: string,
	bookId: string,
	annotationId: string,
	participantId: string,
	input: ClubReaderAnnotationPatch
): Promise<ClubReaderAnnotation> {
	return requestJson<ClubReaderAnnotation>(
		`${bookPath(sessionId, bookId)}/annotations/${encodeURIComponent(annotationId)}`,
		{
			method: 'PATCH',
			body: JSON.stringify(input),
			headers: { 'X-Club-Reader-Participant': participantId }
		}
	);
}

export function deleteClubReaderAnnotation(
	sessionId: string,
	bookId: string,
	annotationId: string,
	participantId: string
): Promise<void> {
	return requestNoContent(
		`${bookPath(sessionId, bookId)}/annotations/${encodeURIComponent(annotationId)}`,
		{ method: 'DELETE', headers: { 'X-Club-Reader-Participant': participantId } }
	);
}

export function listClubReaderMessages(
	sessionId: string,
	options: { before?: string; limit?: number } = {},
	signal?: AbortSignal
): Promise<ClubReaderMessagePage> {
	const query = new URLSearchParams();
	if (options.before) query.set('before', options.before);
	if (options.limit !== undefined) query.set('limit', String(options.limit));
	const search = query.size > 0 ? `?${query}` : '';
	return requestJson<ClubReaderMessagePage>(`${sessionPath(sessionId)}/messages${search}`, { signal });
}

export function postClubReaderMessage(
	sessionId: string,
	participantId: string,
	input: { body: string; bookId?: string }
): Promise<ClubReaderMessage> {
	return requestJson<ClubReaderMessage>(`${sessionPath(sessionId)}/messages`, {
		method: 'POST',
		body: JSON.stringify(input),
		headers: { 'X-Club-Reader-Participant': participantId }
	});
}

export function updateClubReaderMessage(
	sessionId: string,
	messageId: string,
	participantId: string,
	body: string
): Promise<ClubReaderMessage> {
	return requestJson<ClubReaderMessage>(`${sessionPath(sessionId)}/messages/${encodeURIComponent(messageId)}`, {
		method: 'PATCH',
		body: JSON.stringify({ body }),
		headers: { 'X-Club-Reader-Participant': participantId }
	});
}

export function deleteClubReaderMessage(
	sessionId: string,
	messageId: string,
	participantId: string
): Promise<void> {
	return requestNoContent(`${sessionPath(sessionId)}/messages/${encodeURIComponent(messageId)}`, {
		method: 'DELETE',
		headers: { 'X-Club-Reader-Participant': participantId }
	});
}

export interface ClubReaderEventHandlers {
	/** The stream is connected; polling can stop. */
	onOpen(): void;
	/** Something in the session changed; refetch through the authorized GETs. */
	onChanged(kinds: ClubReaderChangeKind[]): void;
	/** The server ended the stream because this reader lost access. */
	onRevoked(): void;
	/**
	 * The stream dropped. `retrying` is true while the browser reconnects on its
	 * own; false once it gave up (non-200 response such as 401 or 429).
	 */
	onDisconnected(retrying: boolean): void;
}

function parseChangeKinds(data: string): ClubReaderChangeKind[] {
	try {
		const payload = JSON.parse(data) as { kinds?: unknown };
		if (Array.isArray(payload.kinds)) {
			const kinds = CHANGE_KINDS.filter((kind) => (payload.kinds as unknown[]).includes(kind));
			if (kinds.length > 0) return kinds;
		}
	} catch {
		// A malformed hint still means "something changed": refetch everything.
	}
	return [...CHANGE_KINDS];
}

/**
 * Opens the cookie-authenticated live stream for a session. Events carry only
 * change kinds, never content. Returns a function that closes the stream.
 */
export function subscribeClubReaderEvents(sessionId: string, handlers: ClubReaderEventHandlers): () => void {
	const source = new EventSource(`${sessionPath(sessionId)}/events`);
	let closed = false;
	const close = () => {
		closed = true;
		source.close();
	};
	source.onopen = () => {
		if (!closed) handlers.onOpen();
	};
	source.addEventListener('changed', (event) => {
		if (!closed) handlers.onChanged(parseChangeKinds((event as MessageEvent<string>).data));
	});
	source.addEventListener('revoked', () => {
		if (closed) return;
		close();
		handlers.onRevoked();
	});
	source.onerror = () => {
		if (closed) return;
		const retrying = source.readyState === EventSource.CONNECTING;
		if (!retrying) close();
		handlers.onDisconnected(retrying);
	};
	return close;
}

export function clubReaderPageUrl(sessionId: string, bookId: string, page: number): string {
	return `${bookPath(sessionId, bookId)}/page/${page}`;
}

export function clubReaderAudioUrl(sessionId: string, bookId: string, trackIndex: number): string {
	return `${bookPath(sessionId, bookId)}/audio/${trackIndex}`;
}

export function createClubReaderResources(sessionId: string, bookId: string): ReaderResourceAdapter {
	const basePath = bookPath(sessionId, bookId);
	const resourcePrefix = new URL(`${basePath}/resource/`, window.location.origin).href;
	return {
		openPublication: async (_mediaId, signal): Promise<Publication> => {
			const manifest = await requestJson<RwpmManifest>(`${basePath}/manifest.json`, {
				signal,
				headers: { Accept: 'application/webpub+json' }
			});
			if (!manifest.readingOrder?.length) {
				throw new Error('The current book has no reading order in its Readium manifest.');
			}
			const positionsResult = await requestJson<RwpmPositions>(`${basePath}/positions.json`, {
				signal,
				headers: { Accept: 'application/vnd.readium.position-list+json' }
			});
			if (!positionsResult.positions?.length) {
				throw new Error('The current book has no Readium positions.');
			}
			return { manifest, positions: positionsResult.positions };
		},
		resourceUrl: (_mediaId, packagePath) => `${basePath}/resource/${packageResourcePath(packagePath)}`,
		resourcePrefix
	};
}
