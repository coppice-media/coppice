export interface ReaderSessionParticipant {
	id: string
	displayName: string
	expiresAt: string | null
	revokedAt: string | null
	linkedAccount: boolean
}

export interface ReaderSessionAdmin {
	id: string
	name: string
	expiresAt: string | null
	closedAt: string | null
	publishedBookId: string | null
	publishedBookTitle: string | null
	participants: ReaderSessionParticipant[]
	canManage: boolean
}

export interface ReaderSessionCredential {
	participant: ReaderSessionParticipant
	readerPath: string
	token: string
}

export interface CreateReaderSessionInput {
	name: string
	expiresAt?: string
}

export interface CreateReaderParticipantInput {
	displayName: string
	expiresAt?: string
}

export interface JoinReaderSessionInput {
	displayName: string
	shareProgress: boolean
}

export interface RotateReaderParticipantInput {
	expiresAt?: string
}

/** Organizer view of a session-discussion message; `authorId` is the participant id. */
export interface ReaderSessionMessage {
	id: string
	authorId: string | null
	authorName: string
	bookId: string | null
	body: string
	createdAt: string
	editedAt: string | null
	mine: boolean
}

/** Newest-first page; pass the last message id as `before` for the next page. */
export interface ReaderSessionMessagePage {
	messages: ReaderSessionMessage[]
	hasMore: boolean
}

const sessionCollection = (clubId: string) =>
	`/api/v2/book-clubs/${encodeURIComponent(clubId)}/reader-sessions`

async function request<T>(path: string, method = 'GET', body?: unknown): Promise<T> {
	const response = await fetch(path, {
		method,
		credentials: 'include',
		cache: 'no-store',
		headers: body === undefined ? { Accept: 'application/json' } : {
			Accept: 'application/json',
			'Content-Type': 'application/json'
		},
		...(body === undefined ? {} : { body: JSON.stringify(body) })
	})

	if (!response.ok) {
		let message = 'The server rejected this reader-session action.'
		try {
			const payload = (await response.json()) as { message?: string; error?: string }
			message = payload.message ?? payload.error ?? message
		} catch {
			// Keep the fallback for empty or non-JSON error responses.
		}
		throw new Error(message)
	}

	if (response.status === 204) return undefined as T
	return (await response.json()) as T
}

export const readerSessionsApi = {
	list(clubId: string): Promise<{ sessions: ReaderSessionAdmin[] }> {
		return request<{ sessions: ReaderSessionAdmin[] }>(sessionCollection(clubId))
	},

	create(clubId: string, input: CreateReaderSessionInput): Promise<ReaderSessionAdmin> {
		return request<ReaderSessionAdmin>(sessionCollection(clubId), 'POST', input)
	},

	publish(clubId: string, sessionId: string, bookId: string): Promise<ReaderSessionAdmin> {
		return request<ReaderSessionAdmin>(
			`${sessionCollection(clubId)}/${encodeURIComponent(sessionId)}/publish`,
			'POST',
			{ bookId }
		)
	},

	advance(clubId: string, sessionId: string): Promise<ReaderSessionAdmin> {
		return request<ReaderSessionAdmin>(
			`${sessionCollection(clubId)}/${encodeURIComponent(sessionId)}/advance`,
			'POST'
		)
	},

	addParticipant(
		clubId: string,
		sessionId: string,
		input: CreateReaderParticipantInput
	): Promise<ReaderSessionCredential> {
		return request<ReaderSessionCredential>(
			`${sessionCollection(clubId)}/${encodeURIComponent(sessionId)}/participants`,
			'POST',
			input
		)
	},

	join(
		clubId: string,
		sessionId: string,
		input: JoinReaderSessionInput
	): Promise<ReaderSessionCredential> {
		return request<ReaderSessionCredential>(
			`${sessionCollection(clubId)}/${encodeURIComponent(sessionId)}/join`,
			'POST',
			input
		)
	},

	rotateParticipant(
		clubId: string,
		sessionId: string,
		participantId: string,
		input: RotateReaderParticipantInput
	): Promise<ReaderSessionCredential> {
		return request<ReaderSessionCredential>(
			`${sessionCollection(clubId)}/${encodeURIComponent(sessionId)}/participants/${encodeURIComponent(participantId)}/rotate`,
			'POST',
			input
		)
	},
	revokeParticipant(clubId: string, sessionId: string, participantId: string): Promise<void> {
		return request<void>(
			`${sessionCollection(clubId)}/${encodeURIComponent(sessionId)}/participants/${encodeURIComponent(participantId)}`,
			'DELETE'
		)
	},

	close(clubId: string, sessionId: string): Promise<void> {
		return request<void>(`${sessionCollection(clubId)}/${encodeURIComponent(sessionId)}`, 'DELETE')
	},

	listMessages(clubId: string, sessionId: string, before?: string): Promise<ReaderSessionMessagePage> {
		const query = new URLSearchParams({ limit: '50' })
		if (before) query.set('before', before)
		return request<ReaderSessionMessagePage>(
			`${sessionCollection(clubId)}/${encodeURIComponent(sessionId)}/messages?${query}`
		)
	},

	deleteMessage(clubId: string, sessionId: string, messageId: string): Promise<void> {
		return request<void>(
			`${sessionCollection(clubId)}/${encodeURIComponent(sessionId)}/messages/${encodeURIComponent(messageId)}`,
			'DELETE'
		)
	}
}
