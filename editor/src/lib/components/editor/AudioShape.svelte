<script lang="ts">
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@stump/ui/components/ui/table';
	import { formatBytes, formatDurationMs, humanize } from '$lib/ingest/helpers';

	// One shape for both sources of an audiobook's facts: the probe result a
	// staged item carries (`IngestDropItemAudio`) and the library record's
	// `MediaAudio`. Neither has a mutation, so the panel is read-only.
	export interface AudioTrackRow {
		label: string;
		durationMs: number;
		startOffsetMs: number;
		byteSize: number;
		detail: string | null;
	}
	export interface AudioChapterRow {
		title: string | null;
		startMs: number;
		endMs: number | null;
	}
	export interface AudioFacts {
		durationMs: number;
		codec: string;
		bitrate: number | null;
		sampleRate: number | null;
		channels: number | null;
		chapterSource: string;
		tracks: AudioTrackRow[];
		chapters: AudioChapterRow[];
		cover: string | null;
		assembled: { filename: string; byteSize: number; method: string; faststart: boolean; partsKept: boolean } | null;
	}

	let { audio }: { audio: AudioFacts } = $props();

	const CHAPTER_SOURCE_PROSE: Record<string, string> = {
		per_track: 'Synthesized from file boundaries; the publisher shipped no chapter marks',
		none: 'No chapter marks in the file',
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

	let chapterProvenance = $derived(
		CHAPTER_SOURCE_PROSE[audio.chapterSource.toLowerCase()] ?? humanize(audio.chapterSource)
	);
	let bitrateKbps = $derived(audio.bitrate === null ? null : Math.round(audio.bitrate / 1000));
</script>

<section aria-labelledby="audio-heading" class="flex flex-col gap-4">
	<div>
		<h3 id="audio-heading" class="text-sm font-semibold">Audio</h3>
		<p class="text-xs text-muted-foreground">Read from the file. Track and chapter marks have no editor yet.</p>
	</div>
	<div class="flex flex-wrap items-center gap-2">
		<Badge variant="outline">{formatDurationMs(audio.durationMs)}</Badge>
		<Badge variant="secondary">{audio.codec}{bitrateKbps === null ? '' : ` · ${bitrateKbps} kbps`}</Badge>
		{#if audio.sampleRate !== null}
			<Badge variant="outline">{audio.sampleRate} Hz</Badge>
		{/if}
		{#if audio.channels !== null}
			<Badge variant="outline">{audio.channels} channel{audio.channels === 1 ? '' : 's'}</Badge>
		{/if}
		{#if audio.cover}
			<Badge variant="outline">{audio.cover}</Badge>
		{/if}
	</div>

	{#if audio.assembled}
		<div class="rounded-lg border bg-muted/40 p-3 text-sm">
			<p class="font-medium">Assembled to {audio.assembled.filename}</p>
			<p class="text-xs text-muted-foreground">
				{ASSEMBLE_METHODS[audio.assembled.method] ?? humanize(audio.assembled.method)} · {formatBytes(audio.assembled.byteSize)}
				{audio.assembled.faststart ? ' · faststart' : ''}{audio.assembled.partsKept ? ' · source parts kept' : ''}
			</p>
		</div>
	{/if}

	<div class="flex flex-col gap-2">
		<p class="text-xs font-medium text-muted-foreground">{audio.tracks.length} track{audio.tracks.length === 1 ? '' : 's'}</p>
		{#if audio.tracks.length}
			<div class="max-h-64 overflow-auto rounded-lg border">
				<Table>
					<TableHeader>
						<TableRow>
							<TableHead class="w-10">#</TableHead>
							<TableHead>File</TableHead>
							<TableHead class="text-right">Start</TableHead>
							<TableHead class="text-right">Length</TableHead>
							<TableHead class="text-right">Size</TableHead>
						</TableRow>
					</TableHeader>
					<TableBody>
						{#each audio.tracks as track, index (index)}
							<TableRow>
								<TableCell class="tabular-nums text-muted-foreground">{index + 1}</TableCell>
								<TableCell class="max-w-[16rem]">
									<div class="truncate">{track.label}</div>
									{#if track.detail}
										<div class="truncate text-xs text-muted-foreground">{track.detail}</div>
									{/if}
								</TableCell>
								<TableCell class="text-right tabular-nums">{formatDurationMs(track.startOffsetMs)}</TableCell>
								<TableCell class="text-right tabular-nums">{formatDurationMs(track.durationMs)}</TableCell>
								<TableCell class="text-right tabular-nums text-muted-foreground">{formatBytes(track.byteSize)}</TableCell>
							</TableRow>
						{/each}
					</TableBody>
				</Table>
			</div>
		{/if}
	</div>

	<div class="flex flex-col gap-2">
		<p class="text-xs font-medium text-muted-foreground">
			{audio.chapters.length} chapter{audio.chapters.length === 1 ? '' : 's'} · {chapterProvenance}
		</p>
		{#if audio.chapters.length}
			<div class="max-h-72 overflow-auto rounded-lg border">
				<Table>
					<TableHeader>
						<TableRow>
							<TableHead class="w-10">#</TableHead>
							<TableHead>Title</TableHead>
							<TableHead class="text-right">Start</TableHead>
							<TableHead class="text-right">End</TableHead>
						</TableRow>
					</TableHeader>
					<TableBody>
						{#each audio.chapters as chapter, index (index)}
							<TableRow>
								<TableCell class="tabular-nums text-muted-foreground">{index + 1}</TableCell>
								<TableCell class="max-w-[18rem] truncate">{chapter.title ?? `Chapter ${index + 1}`}</TableCell>
								<TableCell class="text-right tabular-nums">{formatDurationMs(chapter.startMs)}</TableCell>
								<TableCell class="text-right tabular-nums text-muted-foreground">
									{chapter.endMs === null ? '—' : formatDurationMs(chapter.endMs)}
								</TableCell>
							</TableRow>
						{/each}
					</TableBody>
				</Table>
			</div>
		{/if}
	</div>
</section>
