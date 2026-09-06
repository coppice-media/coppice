<script lang="ts">
	/**
	 * Paged image reader for comics and PDFs.
	 *
	 * Pages are streamed from `GET /api/v2/media/{id}/page/{n}`, which takes a
	 * **visible** page number: the server renumbers pages onto the physical
	 * file after the library's duplicate-page `SKIP` marks
	 * (`stump_core::filesystem::media::visible_pages`). The page count comes
	 * from `mediaVisiblePages`, whose length is the visible count, so a book
	 * with skipped pages never offers a page that 404s.
	 */
	import { untrack } from 'svelte';
	import ChevronLeftIcon from '@lucide/svelte/icons/chevron-left';
	import ChevronRightIcon from '@lucide/svelte/icons/chevron-right';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { Progress } from '@stump/ui/components/ui/progress';
	import * as Select from '@stump/ui/components/ui/select';
	import { Skeleton } from '@stump/ui/components/ui/skeleton';
	import { decimal, type ReaderLocator } from './locator';
	import { pageUrl } from './rwpm';

	type ReaderAnnotation = { id: string; annotationText?: string | null; locator: ReaderLocator };

	let {
		mediaId,
		pageCount,
		startPage = 1,
		annotations = [],
		onPage
	}: {
		mediaId: string;
		pageCount: number;
		startPage?: number;
		annotations?: ReaderAnnotation[];
		onPage: (page: number) => void;
	} = $props();
	const SWIPE_THRESHOLD_PX = 48;

	const FITS = [
		{ value: 'height', label: 'Fit height', class: 'max-h-full w-auto' },
		{ value: 'width', label: 'Fit width', class: 'h-auto w-full' },
		{ value: 'original', label: 'Actual size', class: 'h-auto w-auto max-w-none' }
	];

	// The parent only mounts this reader once the visible page count is known,
	// so the stored page is an initial value, not a tracked one.
	let page = $state(untrack(() => Math.min(Math.max(1, startPage), Math.max(1, pageCount))));
	let fit = $state(FITS[0]!.value);
	let loading = $state(true);
	let failed = $state(false);
	let stage = $state<HTMLDivElement | null>(null);

	const fitClass = $derived(FITS.find((option) => option.value === fit)?.class ?? '');
	const source = $derived(pageUrl(mediaId, page));
	// Annotations on a paged book anchor to a page through the locator's
	// `position`; anything else in the locator has no meaning on an image.
	const pageAnnotations = $derived(
		annotations.filter((annotation) => annotation.locator.locations?.position === page)
	);

	// Preload the neighbours so a page turn is not a blank frame.
	$effect(() => {
		for (const candidate of [page + 1, page - 1]) {
			if (candidate < 1 || candidate > pageCount) continue;
			const image = new Image();
			image.src = pageUrl(mediaId, candidate);
		}
	});

	function goTo(next: number): void {
		const clamped = Math.min(Math.max(1, next), pageCount);
		if (clamped === page) return;
		page = clamped;
		loading = true;
		failed = false;
		onPage(clamped);
	}

	function onKeydown(event: KeyboardEvent): void {
		const target = event.target;
		if (event.metaKey || event.ctrlKey || event.altKey) return;
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
			goTo(page + 1);
		} else if (event.key === 'ArrowLeft' || event.key === 'PageUp') {
			event.preventDefault();
			goTo(page - 1);
		} else if (event.key === 'Home') {
			event.preventDefault();
			goTo(1);
		} else if (event.key === 'End') {
			event.preventDefault();
			goTo(pageCount);
		}
	}

	// Swipe is bound imperatively: the stage is a plain scroll container, and
	// giving it an interactive ARIA role to satisfy the template a11y rule
	// would lie to assistive tech. Paging is reachable from the toolbar
	// buttons and the arrow/Home/End keys, which are the accessible paths.
	$effect(() => {
		const element = stage;
		if (!element) return;

		let swipeStart: { x: number; y: number } | null = null;
		const onTouchStart = (event: TouchEvent) => {
			const touch = event.changedTouches[0];
			swipeStart = touch ? { x: touch.clientX, y: touch.clientY } : null;
		};
		const onTouchEnd = (event: TouchEvent) => {
			const touch = event.changedTouches[0];
			if (!swipeStart || !touch) return;
			const dx = touch.clientX - swipeStart.x;
			const dy = touch.clientY - swipeStart.y;
			swipeStart = null;
			if (Math.abs(dx) < SWIPE_THRESHOLD_PX || Math.abs(dx) < Math.abs(dy)) return;
			goTo(page + (dx < 0 ? 1 : -1));
		};

		element.addEventListener('touchstart', onTouchStart, { passive: true });
		element.addEventListener('touchend', onTouchEnd, { passive: true });
		return () => {
			element.removeEventListener('touchstart', onTouchStart);
			element.removeEventListener('touchend', onTouchEnd);
		};
	});
</script>

<svelte:window onkeydown={onKeydown} />

<div class="flex flex-col gap-3">
	<div class="flex flex-wrap items-center gap-2">
		<Button variant="outline" size="sm" onclick={() => goTo(page - 1)} disabled={page <= 1}>
			<ChevronLeftIcon data-icon="inline-start" />
			Previous
		</Button>
		<Button variant="outline" size="sm" onclick={() => goTo(page + 1)} disabled={page >= pageCount}>
			Next
			<ChevronRightIcon data-icon="inline-end" />
		</Button>

		<Select.Root type="single" value={fit} onValueChange={(value) => (fit = value)}>
			<Select.Trigger class="w-36" aria-label="Fit mode">
				{FITS.find((option) => option.value === fit)?.label}
			</Select.Trigger>
			<Select.Content>
				{#each FITS as option (option.value)}
					<Select.Item value={option.value} label={option.label} />
				{/each}
			</Select.Content>
		</Select.Root>

		<div class="ml-auto flex items-center gap-2">
			{#if pageAnnotations.length}
				<Badge variant="secondary">
					{pageAnnotations.length} highlight{pageAnnotations.length === 1 ? '' : 's'}
				</Badge>
			{/if}
			<span class="text-sm text-muted-foreground">Page {page} of {pageCount}</span>
		</div>
	</div>

	<Progress value={(page / Math.max(1, pageCount)) * 100} />

	<div
		bind:this={stage}
		class="relative flex h-[70vh] min-h-[420px] items-center justify-center overflow-auto rounded-xl border bg-black/90"
	>
		{#if loading && !failed}
			<Skeleton class="absolute inset-8 rounded-lg" />
		{/if}
		{#if failed}
			<Alert variant="destructive" class="m-8 max-w-md">
				<AlertTitle>Page {page} did not load</AlertTitle>
				<AlertDescription>
					The server could not render this page. Try another page or check the file's status.
				</AlertDescription>
			</Alert>
		{:else}
			<img
				src={source}
				alt="Page {page}"
				class="{fitClass} relative select-none"
				draggable="false"
				onload={() => (loading = false)}
				onerror={() => {
					loading = false;
					failed = true;
				}}
			/>
		{/if}

		{#if pageAnnotations.length}
			<ul
				class="absolute top-3 right-3 flex max-w-xs flex-col gap-2"
				aria-label="Highlights on this page"
			>
				{#each pageAnnotations as annotation (annotation.id)}
					<li class="rounded-md bg-amber-400/90 px-2 py-1 text-xs text-amber-950 shadow">
						{annotation.annotationText ||
							annotation.locator.text?.highlight ||
							`Highlight at ${Math.round((decimal(annotation.locator.locations?.totalProgression) ?? 0) * 100)}%`}
					</li>
				{/each}
			</ul>
		{/if}
	</div>

	<p class="text-xs text-muted-foreground">
		Arrow keys, Home/End, or swipe to turn the page. Highlights are read-only here.
	</p>
</div>
