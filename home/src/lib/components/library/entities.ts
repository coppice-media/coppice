import type { MediaFilterInput } from '$lib/graphql/generated/graphql';

/** One tile of an entity grid (author, publisher, or tag). */
export type EntityTileData = {
	key: string;
	name: string;
	/** Right-aligned line under the name, e.g. `12 series`. */
	detail?: string;
	/**
	 * Book count when the query already knows it. Tiles without one ask for it
	 * themselves: neither `mediaMetadataOverview` nor `tags` aggregates books.
	 */
	bookCount?: number;
	/** Filter the count query — and the library screen the tile links to — use. */
	filter?: MediaFilterInput;
	/** Where the tile navigates: the library browser, pre-filtered. */
	href: string;
};
