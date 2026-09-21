<script lang="ts">
	import AudioLinesIcon from '@lucide/svelte/icons/audio-lines';
	import CheckCircle2Icon from '@lucide/svelte/icons/check-circle-2';
	import CircleOffIcon from '@lucide/svelte/icons/circle-off';
	import Clock3Icon from '@lucide/svelte/icons/clock-3';
	import FileAudioIcon from '@lucide/svelte/icons/file-audio';
	import Link2Icon from '@lucide/svelte/icons/link-2';
	import TriangleAlertIcon from '@lucide/svelte/icons/triangle-alert';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@stump/ui/components/ui/card';
	import type { BookDetail, ReadAloud } from '$lib/book/detail';
	import { formatDuration } from '$lib/book/detail';

	let { detail, readAloud }: { detail: BookDetail; readAloud?: ReadAloud | null } = $props();
	const audioEdition = $derived(detail.editions.find((edition) => edition.kind === 'AUDIOBOOK') ?? null);
	const ebookEdition = $derived(detail.editions.find((edition) => edition.kind === 'EBOOK') ?? null);
	const status = $derived(readAloud?.status ?? (audioEdition ? 'UNAVAILABLE' : 'NO_AUDIOBOOK_PAIRED'));
	const label = $derived.by(() => {
		switch (status) {
			case 'NO_AUDIOBOOK_PAIRED': return 'No audiobook paired';
			case 'READY': return 'Ready for read-aloud';
			case 'CHAPTER_MAP_UNAVAILABLE': return 'Chapter map unavailable';
			case 'SYNC_MAP_UNAVAILABLE': return 'Sync map unavailable';
			case 'FAILED': return 'Read-aloud failed';
			case 'UNAVAILABLE': return 'Read-aloud unavailable';
			case 'CACHE_MISSING': return 'Cached artifact unavailable';
			default: return 'Read-aloud status unavailable';
		}
	});
	const isReady = $derived(status === 'READY');

	function statusVariant(): 'secondary' | 'destructive' | 'outline' {
		if (isReady) return 'secondary';
		if (status === 'FAILED') return 'destructive';
		return 'outline';
	}
</script>

<Card>
	<CardHeader>
		<div class="flex flex-wrap items-start gap-3">
			<div class="mr-auto">
				<CardTitle class="flex items-center gap-2"><AudioLinesIcon class="size-4" aria-hidden="true" />Read-aloud</CardTitle>
				<CardDescription>Audio alignment is reported from local files and persisted maps, never inferred from titles.</CardDescription>
			</div>
			<Badge variant={statusVariant()}>{label}</Badge>
		</div>
	</CardHeader>
	<CardContent class="flex flex-col gap-4">
		{#if status === 'NO_AUDIOBOOK_PAIRED'}
			<div class="flex items-start gap-3 rounded-lg border border-dashed p-4">
				<CircleOffIcon class="mt-0.5 size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
				<div><p class="text-sm font-medium">No audiobook paired</p><p class="text-xs text-muted-foreground">Pair an audiobook edition from Files & editions before asking for read-aloud alignment.</p></div>
			</div>
		{:else if status === 'FAILED'}
			<Alert variant="destructive"><TriangleAlertIcon class="size-4" aria-hidden="true" /><AlertTitle>Alignment failed</AlertTitle><AlertDescription>{readAloud?.reason ?? 'The server did not produce a usable alignment artifact.'}</AlertDescription></Alert>
		{:else}
			<div class="grid gap-3 sm:grid-cols-2">
				<div class="rounded-lg border p-3">
					<p class="flex items-center gap-2 text-xs text-muted-foreground"><FileAudioIcon class="size-3.5" aria-hidden="true" />Measured runtime</p>
					<p class="mt-1 text-lg font-semibold">{formatDuration(audioEdition?.audio?.durationMs)}</p>
					<p class="text-xs text-muted-foreground">{audioEdition?.audio?.codec?.toUpperCase() ?? 'Codec unavailable'}</p>
				</div>
				<div class="rounded-lg border p-3">
					<p class="flex items-center gap-2 text-xs text-muted-foreground"><Clock3Icon class="size-3.5" aria-hidden="true" />Chapters</p>
					<p class="mt-1 text-lg font-semibold">{audioEdition?.audio?.chapters.length ?? 0}</p>
					<p class="text-xs text-muted-foreground">{audioEdition?.audio?.chapterSource ?? 'Chapter data unavailable'}</p>
				</div>
			</div>
			{#if !audioEdition?.audio}
				<p class="rounded-lg border border-dashed p-3 text-sm text-muted-foreground">Runtime, chapters, and abridged status are unavailable because no measured audio facts were returned.</p>
			{:else if audioEdition.audio.chapters.length === 0}
				<p class="rounded-lg border border-dashed p-3 text-sm text-muted-foreground">Runtime is measured locally. Chapter marks are unavailable; this does not imply an abridged or unabridged edition.</p>
			{/if}
			{#if readAloud?.reason && !isReady}<p class="text-sm text-muted-foreground">{readAloud.reason}</p>{/if}
			<div class="flex flex-wrap gap-2 text-xs text-muted-foreground">
				<span class="inline-flex items-center gap-1"><Link2Icon class="size-3" aria-hidden="true" />Ebook: {ebookEdition?.mediaId ?? 'Unavailable'}</span>
				<span class="inline-flex items-center gap-1"><Link2Icon class="size-3" aria-hidden="true" />Audiobook: {audioEdition?.mediaId ?? 'Unavailable'}</span>
			</div>
			{#if readAloud?.chapterMap.length}
				<p class="flex items-center gap-2 text-sm"><CheckCircle2Icon class="size-4 text-emerald-500" aria-hidden="true" />{readAloud.chapterMap.length} chapter map entries</p>
			{/if}
			{#if readAloud?.syncMap}
				<p class="text-xs text-muted-foreground">Sync map · {readAloud.syncMap.generator} {readAloud.syncMap.generatorVersion} · {readAloud.syncMap.cueCount} cues{readAloud.syncMap.coverage == null ? '' : ` · ${Math.round(readAloud.syncMap.coverage * 100)}% coverage`}</p>
			{/if}
			{#if readAloud?.artifact}
				<a class="text-xs text-primary underline-offset-4 hover:underline" href={readAloud.artifact.url} target="_blank" rel="noreferrer">Open cached read-aloud artifact</a>
			{/if}
		{/if}
	</CardContent>
</Card>
