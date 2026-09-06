import { browser } from '$app/environment';
import { goto } from '$app/navigation';
import { resolve } from '$app/paths';
import { page } from '$app/state';
import { createQuery } from '@tanstack/svelte-query';
import { request } from '@stump/ui/graphql/client';
import { ConsoleLibraryOptionsDocument } from '$lib/graphql/generated/graphql';
import type { OffsetPage } from '$lib/library';

export type LibraryOption = { id: string; name: string; emoji?: string | null };

/**
 * The shared state of the entity screens: which library the grid is about,
 * what the user typed, and which page they are on. It lives in the URL so a
 * tile's link, the browser's back button, and a reload all agree.
 *
 * A library is always resolved (the first one until the user picks another),
 * because every tile links into that library's book browser.
 */
export type EntityScope = {
	readonly libraries: LibraryOption[];
	readonly libraryId: string | null;
	readonly library: LibraryOption | null;
	readonly search: string | null;
	readonly page: number;
	readonly isPending: boolean;
	readonly error: unknown;
	setLibrary: (id: string) => void;
	setSearch: (value: string | null) => void;
	setPage: (value: number) => void;
};

export function useEntityScope(): EntityScope {
	const librariesQuery = createQuery(() => ({
		queryKey: ['libraryOptions'],
		queryFn: () => request(ConsoleLibraryOptionsDocument, {}),
		enabled: browser
	}));

	const libraries = $derived(librariesQuery.data?.libraries.nodes ?? []);
	const params = $derived(page.url.searchParams);
	const requested = $derived(params.get('library'));
	const libraryId = $derived(
		(requested && libraries.some((entry) => entry.id === requested) ? requested : null) ??
			libraries[0]?.id ??
			null
	);
	const library = $derived(libraries.find((entry) => entry.id === libraryId) ?? null);
	const search = $derived(params.get('q'));
	const pageNumber = $derived(Math.max(1, Number(params.get('page') ?? '1') || 1));

	function navigate(patch: Record<string, string | null>, resetPage = true): void {
		const url = new URL(page.url);
		for (const [key, value] of Object.entries(patch)) {
			if (value === null || value === '') url.searchParams.delete(key);
			else url.searchParams.set(key, value);
		}
		if (resetPage) url.searchParams.delete('page');
		void goto(url, { replaceState: true, noScroll: true, keepFocus: true });
	}

	return {
		get libraries() {
			return libraries;
		},
		get libraryId() {
			return libraryId;
		},
		get library() {
			return library;
		},
		get search() {
			return search;
		},
		get page() {
			return pageNumber;
		},
		get isPending() {
			return librariesQuery.isPending;
		},
		get error() {
			return librariesQuery.isError ? librariesQuery.error : null;
		},
		setLibrary: (id: string) => navigate({ library: id }),
		setSearch: (value: string | null) => navigate({ q: value }),
		setPage: (value: number) => navigate({ page: String(value) }, false)
	};
}

/** The link behind an entity tile: this library's books, pre-filtered. */
export function entityHref(
	scope: EntityScope,
	facet: { author?: string; publisher?: string; tag?: string }
): string {
	if (!scope.libraryId) return resolve('/library');
	const query = new URLSearchParams({ tab: 'books' });
	for (const [key, value] of Object.entries(facet)) {
		if (value) query.set(key, value);
	}
	return `${resolve('/(app)/library/[id]', { id: scope.libraryId })}?${query}`;
}

/** Client-side paging for the entity lists the server returns whole. */
export function slicePage<T>(
	items: T[],
	pageNumber: number,
	pageSize: number
): { items: T[]; pageInfo: OffsetPage } {
	const totalPages = Math.max(1, Math.ceil(items.length / pageSize));
	const currentPage = Math.min(pageNumber, totalPages);
	const start = (currentPage - 1) * pageSize;
	return {
		items: items.slice(start, start + pageSize),
		pageInfo: { totalItems: items.length, totalPages, currentPage, pageSize }
	};
}
