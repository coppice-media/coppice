<script lang="ts">
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@stump/ui/components/ui/card';
	import { Input } from '@stump/ui/components/ui/input';
	import { Label } from '@stump/ui/components/ui/label';
	import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@stump/ui/components/ui/table';
	import type {
		IngestItemQuery,
		IngestMetadataFieldSelectionInput,
		MetadataField
	} from '$lib/graphql/generated/graphql';
	import { formatBytes, formatDurationMs, humanize, parseJsonObject } from '$lib/ingest/helpers';

	type Audio = NonNullable<NonNullable<IngestItemQuery['ingestItem']>['audio']>;

	let {
		audio,
		pendingFields,
		onsave,
		disabled = false
	}: {
		audio: Audio;
		pendingFields: unknown;
		onsave: (selections: IngestMetadataFieldSelectionInput[]) => void;
		disabled?: boolean;
	} = $props();

	type EditableKey = 'title' | 'authors' | 'narrator' | 'series' | 'year';
	type Editable = {
		key: EditableKey;
		label: string;
		// The public `MetadataField` the apply path speaks. `WRITERS` is the
		// canonical spelling of the ingest `Authors` fold, and `NARRATORS` is
		// the audiobook reader credit -- a separate column from the writers.
		field: MetadataField;
		// The key `pending_fields` stores this field under: the *ingest* field
		// name, which differs from the public one for authors and dates.
		pendingKey: string;
		// `WRITERS`/`NARRATORS` take a JSON array; the rest take a string.
		list: boolean;
		placeholder: string;
		probeHint: string;
	};

	const EDITABLE: readonly Editable[] = [
		{ key: 'title', label: 'Title', field: 'TITLE', pendingKey: 'TITLE', list: false, placeholder: 'Audiobook title', probeHint: 'from the file' },
		{ key: 'authors', label: 'Authors', field: 'WRITERS', pendingKey: 'AUTHORS', list: true, placeholder: 'Comma separated', probeHint: 'from the file' },
		{ key: 'narrator', label: 'Narrator', field: 'NARRATORS', pendingKey: 'NARRATORS', list: true, placeholder: 'Comma separated', probeHint: 'from the file' },
		{ key: 'series', label: 'Series', field: 'SERIES', pendingKey: 'SERIES', list: false, placeholder: 'Series name', probeHint: 'from the file (album tag)' },
		{ key: 'year', label: 'Year', field: 'YEAR', pendingKey: 'PUBLISHED_DATE', list: false, placeholder: 'YYYY', probeHint: 'from the file' }
	];

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

	function probeValue(key: EditableKey): string {
		switch (key) {
			case 'title':
				return audio.title ?? '';
			case 'authors':
				return audio.author ?? '';
			case 'narrator':
				return audio.narrator ?? '';
			case 'series':
				// An audiobook container carries no series field; publishers put
				// the series in the album tag, so that is the probe's answer.
				return audio.album ?? '';
			case 'year':
				return audio.year === null || audio.year === undefined ? '' : String(audio.year);
		}
	}

	function pendingText(value: unknown, list: boolean): string {
		if (value === null || value === undefined) return '';
		if (list) {
			return Array.isArray(value)
				? value.filter((entry): entry is string => typeof entry === 'string').join(', ')
				: typeof value === 'string'
					? value
					: '';
		}
		return typeof value === 'string' ? value : String(value);
	}

	let baseline = $derived.by(() => {
		const pending = parseJsonObject(pendingFields);
		const values = {} as Record<EditableKey, string>;
		const staged = {} as Record<EditableKey, boolean>;
		for (const descriptor of EDITABLE) {
			const isStaged = Object.hasOwn(pending, descriptor.pendingKey);
			staged[descriptor.key] = isStaged;
			values[descriptor.key] = isStaged
				? pendingText(pending[descriptor.pendingKey], descriptor.list)
				: probeValue(descriptor.key);
		}
		return { values, staged };
	});

	let drafts = $state<Record<EditableKey, string>>({
		title: '',
		authors: '',
		narrator: '',
		series: '',
		year: ''
	});
	// Plain `let`, deliberately not `$state`: the effect writes it, and a
	// reactive write would re-trigger the effect that produced it.
	let syncedKey = '';

	$effect(() => {
		const key = JSON.stringify(baseline.values);
		if (key === syncedKey) return;
		syncedKey = key;
		drafts = { ...baseline.values };
	});

	let changed = $derived(
		EDITABLE.some((descriptor) => drafts[descriptor.key].trim() !== baseline.values[descriptor.key].trim())
	);
	let tracks = $derived(audio.tracks);
	let chapters = $derived(audio.chapters);
	let chapterProvenance = $derived(CHAPTER_SOURCE_PROSE[audio.chapterSource] ?? humanize(audio.chapterSource));
	let bitrateKbps = $derived(audio.bitrate === null || audio.bitrate === undefined ? null : Math.round(audio.bitrate / 1000));

	function save(): void {
		const selections: IngestMetadataFieldSelectionInput[] = [];
		for (const descriptor of EDITABLE) {
			const next = drafts[descriptor.key].trim();
			if (next === baseline.values[descriptor.key].trim()) continue;
			if (!next) {
				selections.push({ field: descriptor.field, mode: 'CLEAR' });
				continue;
			}
			selections.push({
				field: descriptor.field,
				mode: 'MANUAL',
				value: descriptor.list
					? next.split(',').map((part) => part.trim()).filter(Boolean)
					: next
			});
		}
		if (selections.length) onsave(selections);
	}
</script>

<Card>
	<CardHeader>
		<CardTitle>Audio</CardTitle>
		<CardDescription>What the probe read out of the staged file. Metadata here is the file's own, not a provider's, until you save an edit.</CardDescription>
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

		<div class="flex flex-col gap-3 border-t pt-4">
			<h3 class="text-sm font-semibold">Metadata</h3>
			{#each EDITABLE as descriptor (descriptor.key)}
				<div class="grid gap-1 lg:grid-cols-[minmax(8rem,0.4fr)_1fr] lg:items-center">
					<div>
						<Label for={`audio-${descriptor.key}`}>{descriptor.label}</Label>
						{#if !baseline.staged[descriptor.key] && baseline.values[descriptor.key]}
							<p class="text-xs text-muted-foreground">{descriptor.probeHint}</p>
						{/if}
					</div>
					<Input
						id={`audio-${descriptor.key}`}
						placeholder={descriptor.placeholder}
						disabled={disabled}
						bind:value={drafts[descriptor.key]}
					/>
				</div>
			{/each}
			<Button class="self-end" disabled={disabled || !changed} onclick={save}>Save</Button>
		</div>
	</CardContent>
</Card>
