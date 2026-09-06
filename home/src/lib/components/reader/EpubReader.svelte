<script lang="ts">
	/**
	 * EPUB reader: foliate-js rendering a publication streamed from the
	 * server's Readium routes, with stored annotations drawn as overlays.
	 *
	 * Position is reported to the parent as a Readium locator whose `href` is
	 * the package-relative resource path and whose `locations` carry the
	 * resource-local progression, the whole-publication progression, and the
	 * resource's `position` from the server's positions list.
	 */
	import { untrack } from 'svelte';
	import ChevronLeftIcon from '@lucide/svelte/icons/chevron-left';
	import ChevronRightIcon from '@lucide/svelte/icons/chevron-right';
	import { Overlayer } from 'foliate-js/overlayer.js';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { Progress } from '@stump/ui/components/ui/progress';
	import * as Select from '@stump/ui/components/ui/select';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import type { ReadiumLocatorInput } from '$lib/graphql/generated/graphql';
	import {
		annotationRange,
		locatorInputFor,
		resolveInitialAnchor,
		type ReaderLocator
	} from './locator';
	import {
		ANNOTATION_HREF,
		makeReadiumBook,
		type ReaderTocItem,
		type ReadiumBook
	} from './readiumBook';
	import { openPublication, packagePathFromHref } from './rwpm';

	type ReaderAnnotation = { id: string; annotationText?: string | null; locator: ReaderLocator };

	let {
		mediaId,
		storedLocator = null,
		storedPercentage,
		annotations = [],
		onLocator
	}: {
		mediaId: string;
		storedLocator?: ReaderLocator | null;
		storedPercentage?: number;
		annotations?: ReaderAnnotation[];
		onLocator: (payload: {
			locator: ReadiumLocatorInput;
			percentage: number;
			isComplete: boolean;
		}) => void;
	} = $props();

	const FLOWS = [
		{ value: 'paginated', label: 'Paged' },
		{ value: 'scrolled', label: 'Scrolling' }
	];

	let host = $state<HTMLDivElement | null>(null);
	let view = $state<FoliateView | null>(null);
	let book = $state<ReadiumBook | null>(null);
	let error = $state<string | null>(null);
	let ready = $state(false);
	let flow = $state(FLOWS[0]!.value);
	let chapter = $state('');
	let fraction = $state(0);
	/** The section whose annotation overlay is live, from `create-overlay`. */
	let overlaidIndex = $state(-1);
	let drawn = $state(0);
	let unresolved = $state(0);

	const flatToc = $derived.by(() => {
		const flatten = (items: ReaderTocItem[], depth: number): { label: string; href: string }[] =>
			items.flatMap((item) => [
				{ label: `${'\u2007\u2007'.repeat(depth)}${item.label}`, href: item.href },
				...flatten(item.subitems ?? [], depth + 1)
			]);
		return flatten(book?.toc ?? [], 0);
	});

	// Opening is keyed on the book alone: the stored position and the reading
	// flow are read untracked so a background refetch or a flow change never
	// tears the renderer down and reopens the publication.
	$effect(() => {
		const container = host;
		const id = mediaId;
		if (!container) return;

		const controller = new AbortController();
		let created: FoliateView | null = null;
		let opened: ReadiumBook | null = null;

		const open = async () => {
			await import('foliate-js/view.js');
			const publication = await openPublication(id, controller.signal);
			if (controller.signal.aborted) return;

			opened = makeReadiumBook(id, publication);
			created = document.createElement('foliate-view');
			created.className = 'block h-full w-full';
			created.addEventListener('draw-annotation', onDrawAnnotation);
			created.addEventListener('create-overlay', onCreateOverlay);
			created.addEventListener('load', onSectionLoad);
			container.append(created);

			await created.open(opened);
			// The renderer's own `relocate` carries the section index, the
			// section-local fraction of the *start* of the visible page, and
			// that page's share of the section. `<foliate-view>`'s relocate
			// only exposes a whole-publication fraction measured at the page's
			// end, which cannot be fed back as an opening anchor without
			// drifting a page forward on every restore.
			created.renderer.addEventListener('relocate', onRelocate);
			created.renderer.setAttribute('flow', untrack(() => flow));
			created.renderer.setAttribute('margin', '32px');
			created.renderer.setAttribute('gap', '6%');
			created.renderer.setAttribute('max-inline-size', '720px');
			created.renderer.setAttribute('max-block-size', '1400px');

			// Published before the opening navigation so `onRelocate` can render
			// the restored position; `ready` gates *persisting* it, because
			// re-opening a book is not a new reading position.
			view = created;
			book = opened;

			const anchor = untrack(() =>
				resolveInitialAnchor({
					packagePaths: opened!.packagePaths,
					positions: publication.positions,
					locator: storedLocator,
					percentage: storedPercentage
				})
			);
			await created.renderer.goTo(anchor ?? { index: 0, anchor: () => 0 });
			ready = true;
		};

		open().catch((cause: unknown) => {
			if (controller.signal.aborted) return;
			error = cause instanceof Error ? cause.message : 'Unable to open this EPUB.';
		});

		return () => {
			controller.abort();
			created?.close();
			created?.remove();
			opened?.destroy();
			view = null;
			book = null;
			ready = false;
			overlaidIndex = -1;
		};
	});

	/**
	 * Persist the start of the visible page as a Readium locator.
	 *
	 * `totalProgression` is the section's own slice of the publication scale,
	 * whose weights come from the server's positions list, so it matches the
	 * percentage the server would compute for the same resource. Completion
	 * uses the *end* of the visible page: the last page of a book starts
	 * before 100%.
	 */
	function onRelocate(event: Event): void {
		const detail = (event as CustomEvent<FoliateRendererRelocateDetail>).detail;
		const current = book?.sections[detail.index];
		const reader = view;
		if (!current || !reader) return;

		const starts = reader.getSectionFractions();
		const start = starts[detail.index] ?? 0;
		const span = (starts[detail.index + 1] ?? 1) - start;
		const progression = detail.fraction ?? 0;
		const pageEnd = Math.min(1, progression + (detail.size ?? 0));
		const label = reader.getProgressOf(detail.index, detail.range ?? undefined)?.tocItem?.label;

		fraction = start + progression * span;
		chapter = label ?? current.title ?? '';

		if (!ready) return;

		onLocator({
			locator: locatorInputFor({
				packagePath: current.packagePath,
				mediaType: current.mediaType,
				chapterTitle: label ?? current.title,
				title: current.title,
				progression,
				totalProgression: fraction,
				position: current.position
			}),
			percentage: fraction,
			isComplete: start + pageEnd * span >= 0.999
		});
	}

	function onDrawAnnotation(event: Event): void {
		const detail = (event as CustomEvent<FoliateDrawAnnotationDetail>).detail;
		detail.draw(Overlayer.highlight, { color: '#f59e0b' });
	}

	/**
	 * `create-overlay` announces a section's annotation layer. foliate calls
	 * `attach()` right after emitting it, so the layer is only reachable
	 * through `getContents()` on the next microtask.
	 */
	function onCreateOverlay(event: Event): void {
		const { index } = (event as CustomEvent<{ index: number }>).detail;
		queueMicrotask(() => (overlaidIndex = index));
	}

	/**
	 * Draw the stored annotations that live in the section now on screen.
	 *
	 * A locator anchors to a resource plus a quote or selector, so it can only
	 * be resolved against the document that is actually live in the renderer.
	 */
	$effect(() => {
		const reader = view;
		const publication = book;
		const list = annotations;
		const index = overlaidIndex;
		if (!reader || !publication || index < 0) return;

		const contents = reader.renderer.getContents().find((item) => item.index === index);
		if (!contents?.overlayer) return;

		let resolved = 0;
		let skipped = 0;
		for (const annotation of list) {
			if (packagePathFromHref(annotation.locator.href) !== publication.packagePaths[index]) continue;
			if (!annotationRange(contents.doc, annotation.locator)) {
				skipped += 1;
				continue;
			}
			publication.annotationLocators.set(annotation.id, annotation.locator);
			resolved += 1;
			void reader.addAnnotation({ value: `${ANNOTATION_HREF}${annotation.id}` }).catch((cause) => {
				unresolved += 1;
				console.warn(`[reader] highlight ${annotation.id} could not be drawn`, cause);
			});
		}
		drawn = resolved;
		unresolved = skipped;
	});

	function setFlow(value: string): void {
		flow = value;
		view?.renderer.setAttribute('flow', value);
	}

	/**
	 * Paging keys. Also bound to every section document, because clicking the
	 * text moves focus into the renderer's iframe and the window then never
	 * sees the keystroke.
	 */
	function onKeydown(event: KeyboardEvent): void {
		const target = event.target;
		if (!view || event.metaKey || event.ctrlKey || event.altKey) return;
		if (
			target instanceof HTMLElement &&
			(target.isContentEditable ||
				/^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName) ||
				target.closest('[aria-expanded="true"], [role="listbox"]'))
		) {
			return;
		}
		if (event.key === 'ArrowRight' || event.key === 'PageDown') {
			event.preventDefault();
			void view.next();
		} else if (event.key === 'ArrowLeft' || event.key === 'PageUp') {
			event.preventDefault();
			void view.prev();
		}
	}

	function onSectionLoad(event: Event): void {
		const { doc } = (event as CustomEvent<{ doc: Document }>).detail;
		doc.addEventListener('keydown', onKeydown);
	}
</script>

<svelte:window onkeydown={onKeydown} />

<div class="flex flex-col gap-3">
	<div class="flex flex-wrap items-center gap-2">
		<Button variant="outline" size="sm" onclick={() => view?.prev()} disabled={!ready}>
			<ChevronLeftIcon data-icon="inline-start" />
			Previous
		</Button>
		<Button variant="outline" size="sm" onclick={() => view?.next()} disabled={!ready}>
			Next
			<ChevronRightIcon data-icon="inline-end" />
		</Button>

		{#if flatToc.length}
			<Select.Root
				type="single"
				value=""
				onValueChange={(href) => view?.goTo(href)}
				disabled={!ready}
			>
				<Select.Trigger class="w-56" aria-label="Table of contents">
					{chapter || 'Contents'}
				</Select.Trigger>
				<Select.Content>
					{#each flatToc as item, index (index)}
						<Select.Item value={item.href} label={item.label} />
					{/each}
				</Select.Content>
			</Select.Root>
		{/if}

		<Select.Root type="single" value={flow} onValueChange={setFlow} disabled={!ready}>
			<Select.Trigger class="w-32" aria-label="Reading flow">
				{FLOWS.find((option) => option.value === flow)?.label}
			</Select.Trigger>
			<Select.Content>
				{#each FLOWS as option (option.value)}
					<Select.Item value={option.value} label={option.label} />
				{/each}
			</Select.Content>
		</Select.Root>

		<div class="ml-auto flex items-center gap-2">
			{#if drawn}
				<Badge variant="secondary">{drawn} highlight{drawn === 1 ? '' : 's'}</Badge>
			{/if}
			{#if unresolved}
				<Badge
					variant="outline"
					title="Stored highlights whose quoted text is no longer in this resource"
				>
					{unresolved} unanchored
				</Badge>
			{/if}
			<span class="text-sm text-muted-foreground">{Math.round(fraction * 100)}%</span>
		</div>
	</div>

	<Progress value={fraction * 100} />

	{#if error}
		<Alert variant="destructive">
			<AlertTitle>Unable to open this EPUB</AlertTitle>
			<AlertDescription>{error}</AlertDescription>
		</Alert>
	{:else}
		<div class="relative h-[70vh] min-h-[420px] overflow-hidden rounded-xl border bg-background">
			{#if !ready}
				<div class="absolute inset-0 flex flex-col gap-3 p-8">
					<Skeleton class="h-5 w-2/3" />
					<Skeleton class="h-5 w-full" />
					<Skeleton class="h-5 w-full" />
					<Skeleton class="h-5 w-5/6" />
					<Skeleton class="h-5 w-4/6" />
				</div>
			{/if}
			<div bind:this={host} class="h-full w-full" class:invisible={!ready}></div>
		</div>
	{/if}

	<p class="text-xs text-muted-foreground">
		{chapter ? `${chapter} · ` : ''}Arrow keys or swipe to turn the page. Highlights are read-only
		here.
	</p>
</div>
