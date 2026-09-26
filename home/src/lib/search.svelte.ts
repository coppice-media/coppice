/**
 * The three-query search shared by the header palette and `/search`: the
 * library index answers on its own, the Hardcover lookup answers on its own,
 * the Audible lookup answers on its own, and each surface renders whichever
 * arrives first.
 */
import { browser } from '$app/environment'
import { resolve } from '$app/paths'
import { createQuery } from '@tanstack/svelte-query'
import { request } from '@stump/ui/graphql/client'
import { audioLengthLabel } from '$lib/requests'
import {
	AudibleBookSearchDocument,
	ExternalBookSearchDocument,
	LibrarySearchDocument,
	type ExternalHitFieldsFragment,
	type LibrarySearchQuery,
} from '$lib/graphql/generated/graphql'

export type LibraryHit = LibrarySearchQuery['librarySearch'][number]
export type ExternalHit = ExternalHitFieldsFragment

/** The Audible catalog is the one provider whose hits are audiobook editions. */
export const AUDIBLE_PROVIDER = 'audible'

export const SEARCH_DEBOUNCE_MS = 250
/** The library index is local and cheap; two characters are enough. */
export const LIBRARY_MIN_CHARS = 2
/** Hardcover fuzzes short prefixes (`medi` matches every “Media”), so it waits for a third. */
export const EXTERNAL_MIN_CHARS = 3

/**
 * Search text as typed (`value`) and as searched (`query`): the trimmed text
 * once it has settled for `SEARCH_DEBOUNCE_MS`, or `''` below the minimum.
 */
export class SearchTerm {
	value = $state('')
	query = $state('')
	#timer: ReturnType<typeof setTimeout> | null = null

	constructor(initial = '') {
		this.reset(initial)
	}

	/** The field holds a searchable text that `query` has not caught up with yet. */
	get typing(): boolean {
		const trimmed = this.value.trim()
		return trimmed.length >= LIBRARY_MIN_CHARS && trimmed !== this.query
	}

	/** Follow the field; `query` catches up after the debounce. */
	update(next: string): void {
		this.value = next
		this.#cancel()
		if (next.trim().length < LIBRARY_MIN_CHARS) {
			this.query = ''
			return
		}
		this.#timer = setTimeout(() => {
			this.#timer = null
			this.query = next.trim()
		}, SEARCH_DEBOUNCE_MS)
	}

	/** Apply the field now (Enter, submit) and return the resulting query. */
	flush(): string {
		this.#cancel()
		const trimmed = this.value.trim()
		this.query = trimmed.length >= LIBRARY_MIN_CHARS ? trimmed : ''
		return this.query
	}

	/** Replace both field and query at once, e.g. from the route's `?q`. */
	reset(next = ''): void {
		this.#cancel()
		this.value = next
		this.query = next.trim().length >= LIBRARY_MIN_CHARS ? next.trim() : ''
	}

	#cancel(): void {
		clearTimeout(this.#timer ?? undefined)
		this.#timer = null
	}
}

/**
 * Library hits for `term()`. The key carries the query, so a superseded
 * search is aborted through TanStack's signal instead of racing the new one.
 */
export function createLibrarySearch(term: () => string, limit: number) {
	return createQuery(() => {
		const query = term()
		return {
			queryKey: ['library-search', query, limit],
			queryFn: ({ signal }: { signal: AbortSignal }) =>
				request(LibrarySearchDocument, { query, limit }, { signal }),
			enabled: browser && query.length >= LIBRARY_MIN_CHARS,
			staleTime: 30_000,
		}
	})
}

/** Hardcover hits for `term()`, gated behind `EXTERNAL_MIN_CHARS`. */
export function createExternalSearch(term: () => string, limit: number) {
	return createQuery(() => {
		const query = term()
		return {
			queryKey: ['external-book-search', query, limit],
			queryFn: ({ signal }: { signal: AbortSignal }) =>
				request(ExternalBookSearchDocument, { query, limit }, { signal }),
			enabled: browser && query.length >= EXTERNAL_MIN_CHARS,
			staleTime: 60_000,
		}
	})
}

/**
 * Audible hits for `term()`, gated like Hardcover. Audible only answers with
 * audiobook editions, so a surface lists these as “only on Audible” after
 * `onlyOnAudible` drops the works Hardcover already covers.
 */
export function createAudibleSearch(term: () => string, limit: number) {
	return createQuery(() => {
		const query = term()
		return {
			queryKey: ['audible-book-search', query, limit],
			queryFn: ({ signal }: { signal: AbortSignal }) =>
				request(AudibleBookSearchDocument, { query, limit }, { signal }),
			enabled: browser && query.length >= EXTERNAL_MIN_CHARS,
			staleTime: 60_000,
		}
	})
}

/**
 * `title|first author` with case, accents, punctuation and a subtitle
 * stripped, so the same work from two catalogs lands on one key.
 */
function workKey(hit: { title: string; authors?: string | null }): string {
	const title = hit.title.split(/[:(]/, 1)[0] ?? hit.title
	const author = hit.authors?.split(/[,;&]|\band\b/, 1)[0] ?? ''
	return `${title}|${author}`
		.normalize('NFKD')
		.replace(/[\u0300-\u036f]/g, '')
		.toLowerCase()
		.replace(/[^a-z0-9|]+/g, ' ')
		.replace(/\s*\|\s*/, '|')
		.trim()
}

/** The Audible hits whose work no Hardcover hit already offers, at most `limit`. */
export function onlyOnAudible(
	audible: readonly ExternalHit[],
	hardcover: readonly ExternalHit[],
	limit: number,
): ExternalHit[] {
	const covered: Record<string, true> = {}
	for (const hit of hardcover) covered[workKey(hit)] = true
	return audible.filter((hit) => !covered[workKey(hit)]).slice(0, limit)
}

/** `Jim Dale · 11h 40m`, whichever parts Audible filled in. */
export function audibleHitMeta(hit: ExternalHit): string {
	const narrator = hit.narrators?.[0]
	return [narrator ? `Narr. ${narrator}` : '', audioLengthLabel(hit.audioSeconds)].filter(Boolean).join(' · ')
}

/**
 * Where opening a Hardcover hit leads: the library copy when there is one,
 * the existing request otherwise, else the request form seeded with the
 * allow-listed metadata handoff.
 */
export function externalHitHref(hit: ExternalHit): string {
	if (hit.inLibraryMediaIds.length) {
		return resolve('/(app)/book/[mediaId]', { mediaId: hit.inLibraryMediaIds[0] })
	}
	if (hit.existingRequestId) {
		return resolve('/(app)/requests/[id]', { id: hit.existingRequestId })
	}
	const params = new URLSearchParams({
		provider: hit.provider,
		remoteId: hit.remoteId,
		title: hit.title,
	})
	if (hit.authors) params.set('authors', hit.authors)
	if (hit.coverUrl) params.set('coverUrl', hit.coverUrl)
	// An Audible edition is an audiobook by definition; its narrator seeds the preference.
	if (hit.provider === AUDIBLE_PROVIDER) {
		params.set('format', 'AUDIOBOOK')
		if (hit.narrators?.[0]) params.set('narrator', hit.narrators[0])
	}
	return `${resolve('/(app)/requests/new')}?${params}`
}

/** `2019 · Discworld #3`, whichever parts the provider filled in. */
export function externalHitMeta(hit: ExternalHit): string {
	const series = hit.seriesName
		? `${hit.seriesName}${hit.seriesPosition ? ` #${hit.seriesPosition}` : ''}`
		: ''
	return [hit.year, series].filter(Boolean).join(' · ')
}
