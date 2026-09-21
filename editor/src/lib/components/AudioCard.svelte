<script lang="ts">
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@stump/ui/components/ui/card';
	import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@stump/ui/components/ui/table';
	import type { IngestItemQuery } from '$lib/graphql/generated/graphql';
	import { formatBytes, formatDurationMs, humanize } from '$lib/ingest/helpers';

	type Audio = NonNullable<NonNullable<IngestItemQuery['ingestItem']>['audio']>;

	let { audio }: { audio: Audio } = $props();

	const CHAPTER_SOURCE_PROSE: Record<string, string> = {
		per_track: 'Synthesized from file boundaries — the publisher shipped no chapter marks',
		none: 'No chapter marks',
		mp4_chpl: 'Nero chpl atom',
		mp4_chapter_track: 'QuickTime chapter track',
		id3_chap: 'ID3v2 CHAP frames',
		vorbis_comment: 'Vorbis CHAPTER comments',
		container: 'Embedded in the container'
	};

	const ASSEMBLE_METHODS: Record<string, string> = {
		'assemble-remux': 'Lossless remux',
		'assemble-transcode': 'Re-encoded (AAC)'
	};

	let tracks = $derived(audio.tracks);
	let chapters = $derived(audio.chapters);
	let chapterProvenance = $derived(CHAPTER_SOURCE_PROSE[audio.chapterSource] ?? humanize(audio.chapterSource));
	let bitrateKbps = $derived(audio.bitrate === null || audio.bitrate === undefined ? null : Math.round(audio.bitrate / 1000));
</script>

<Card>
	<CardHeader>
		<CardTitle>Audio</CardTitle>
		<CardDescription>Read-only facts from the staged file. Track, chapter, and embedded-cover facts have no metadata mutation.</CardDescription>
	</CardHeader>
	<CardContent class="flex flex-col gap-6">
		<div class="flex flex-wrap items-center gap-2">
			<Badge variant="outline">{audio.duration}</Badge>
			<Badge variant="secondary">{audio.codec}{bitrateKbps === null ? '' : ` · ${bitrateKbps} kbps`}</Badge>
			{#if audio.sampleRate !== null && audio.sampleRate !== undefined}
				<Badge variant="outline">{audio.sampleRate} Hz</Badge>
			{/if}
			{#if audio.channels !== null && audio.channels !== undefined}
				<Badge variant="outline">{audio.channels} channel{audio.channels === 1 ? '' : 's'}</Badge>
			{/if}
			{#if audio.genre}
				<Badge variant="outline">{audio.genre}</Badge>
			{/if}
		</div>

		{#if audio.coverContentType}
			<p class="text-sm">Embedded cover · {audio.coverContentType} · {formatBytes(audio.coverByteSize ?? 0)}</p>
		{:else}
			<p class="text-sm text-muted-foreground">No embedded cover</p>
		{/if}

		{#if audio.description}
			<div>
				<p class="text-sm font-medium">Embedded description</p>
				<p class="mt-1 max-h-24 overflow-auto text-sm text-muted-foreground">{audio.description}</p>
			</div>
		{/if}

		<div class="flex flex-col gap-2">
			<h3 class="text-sm font-semibold">{tracks.length} track{tracks.length === 1 ? '' : 's'}</h3>
			{#if tracks.length}
				<div class="overflow-x-auto rounded-lg border">
					<Table>
						<TableHeader>
							<TableRow>
								<TableHead class="w-12">#</TableHead>
								<TableHead>Name</TableHead>
								<TableHead>Duration</TableHead>
								<TableHead>Starts</TableHead>
								<TableHead>Size</TableHead>
								<TableHead>Codec</TableHead>
							</TableRow>
						</TableHeader>
						<TableBody>
							{#each tracks as track, index (track.filename)}
								<TableRow>
									<TableCell class="tabular-nums">{track.trackNumber ?? index + 1}</TableCell>
									<TableCell class="max-w-[20rem]"><div class="truncate">{track.title ?? track.filename}</div></TableCell>
									<TableCell class="tabular-nums">{track.duration}</TableCell>
									<TableCell class="tabular-nums">{formatDurationMs(track.startOffsetMs)}</TableCell>
									<TableCell class="tabular-nums">{formatBytes(track.byteSize)}</TableCell>
									<TableCell>{track.codec}</TableCell>
								</TableRow>
							{/each}
						</TableBody>
					</Table>
				</div>
			{:else}
				<p class="text-sm text-muted-foreground">A single container; no separate track files.</p>
			{/if}
		</div>

		<div class="flex flex-col gap-2">
			<h3 class="text-sm font-semibold">{chapters.length} chapter{chapters.length === 1 ? '' : 's'}</h3>
			<p class="text-sm text-muted-foreground">{chapterProvenance}</p>
			{#if chapters.length}
				<ol class="flex flex-col gap-1 rounded-lg border p-3">
					{#each chapters as chapter, index (index)}
						<li class="flex items-baseline justify-between gap-3 text-sm">
							<span class="truncate"><span class="text-muted-foreground tabular-nums">{index + 1}.</span> {chapter.title ?? `Chapter ${index + 1}`}</span>
							<span class="shrink-0 text-muted-foreground tabular-nums">{chapter.start}</span>
						</li>
					{/each}
				</ol>
			{/if}
		</div>

		{#if audio.assembled}
			{@const assembled = audio.assembled}
			<div class="flex flex-col gap-2 rounded-lg border p-3">
				<div class="flex flex-wrap items-center justify-between gap-2">
					<h3 class="text-sm font-semibold">Assembled</h3>
					<div class="flex items-center gap-2">
						<Badge variant={assembled.faststart ? 'default' : 'destructive'}>{assembled.faststart ? 'Faststart' : 'Not faststart'}</Badge>
						<Badge variant="secondary">{ASSEMBLE_METHODS[assembled.method] ?? humanize(assembled.method)}</Badge>
					</div>
				</div>
				<p class="truncate text-sm font-medium">{assembled.filename}</p>
				<p class="text-sm text-muted-foreground">{formatBytes(assembled.byteSize)} · {assembled.duration} · {assembled.chapters} chapter{assembled.chapters === 1 ? '' : 's'}</p>
				<p class="text-sm text-muted-foreground">{assembled.partsKept ? 'Source parts kept beside it' : 'Source parts removed'}</p>
			</div>
		{/if}
	</CardContent>
</Card>
