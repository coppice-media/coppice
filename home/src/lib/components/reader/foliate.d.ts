/**
 * Types for the parts of foliate-js (MIT, vendored through bun as the
 * `foliate-js` dependency) that the console reader uses. The package ships
 * plain ES modules with no declarations; only `view.js` (the `<foliate-view>`
 * custom element) and `overlayer.js` (annotation drawing) are consumed.
 *
 * This file is ambient on purpose: the declarations below must stay global so
 * `document.createElement('foliate-view')` is typed without an import.
 */

type FoliateOverlayerRect = {
	left: number;
	top: number;
	right: number;
	bottom: number;
	width: number;
	height: number;
};

type FoliateOverlayerOptions = {
	color?: string;
	width?: number;
	padding?: number;
	radius?: number;
	writingMode?: string;
};

type FoliateOverlayerDraw = (
	rects: FoliateOverlayerRect[],
	options?: FoliateOverlayerOptions
) => SVGElement;

/** A destination inside a section: a fraction of it, an element, or a range. */
type FoliateAnchor = (doc: Document) => number | Element | Range | null;

declare module 'foliate-js/view.js';

declare module 'foliate-js/overlayer.js' {
	export const Overlayer: {
		highlight: FoliateOverlayerDraw;
		underline: FoliateOverlayerDraw;
		squiggly: FoliateOverlayerDraw;
		outline: FoliateOverlayerDraw;
	};
}

interface FoliateRenderer extends HTMLElement {
	goTo(target: { index: number; anchor?: FoliateAnchor }): Promise<void>;
	prev(distance?: number): Promise<void>;
	next(distance?: number): Promise<void>;
	/** The live view; `overlayer` is only set once `create-overlay` attached one. */
	getContents(): { doc: Document; index: number; overlayer?: unknown }[];
	destroy(): void;
}

/**
 * `event.detail` of the renderer's own `relocate`.
 *
 * `fraction` is the section-local position of the *start* of the visible
 * page and `size` that page's share of the section (both absent in scrolled
 * mode until the first measurement).
 */
interface FoliateRendererRelocateDetail {
	reason: string;
	index: number;
	fraction?: number;
	size?: number;
	range?: Range | null;
}

/** `event.detail` of `<foliate-view>`'s `draw-annotation`. */
interface FoliateDrawAnnotationDetail {
	draw(func: FoliateOverlayerDraw, options?: FoliateOverlayerOptions): void;
	annotation: { value: string };
	doc: Document;
	range: Range;
}

interface FoliateView extends HTMLElement {
	renderer: FoliateRenderer;
	open(book: object): Promise<void>;
	close(): void;
	prev(distance?: number): Promise<void>;
	next(distance?: number): Promise<void>;
	goTo(target: string | number): Promise<{ index: number } | undefined>;
	/** Cumulative start fraction of every section, plus a trailing `1`. */
	getSectionFractions(): number[];
	/** TOC and page-list entries covering a place, for a running head. */
	getProgressOf(
		index: number,
		range?: Range
	): { tocItem?: { label: string; href: string } | null } | undefined;
	addAnnotation(annotation: { value: string }): Promise<{ index: number; label: string }>;
}

interface HTMLElementTagNameMap {
	'foliate-view': FoliateView;
}
