<script lang="ts">
	/**
	 * Audiobook player.
	 *
	 * The canonical audiobook is a single-file M4B, but a book may also be a
	 * folder of tracks, and the two have to behave identically: one
	 * publication, one timeline, one saved position. So the element plays
	 * exactly one track at a time and **every** time value this component
	 * exposes — the seek bar, the readout, the chapter marks, the position it
	 * reports — is milliseconds from the start of the *publication*, never
	 * from the start of a file. `locate` and `publicationMs` are the only two
	 * places that conversion happens.
	 *
	 * Tracks stream from `GET /api/v2/media/{id}/audio/track/{index}`, which
	 * is the same-origin path the server already put on
	 * `MediaAudioTrack.url`, so the session cookie rides along.
	 */
	import { onDestroy, untrack } from 'svelte';
	import GaugeIcon from '@lucide/svelte/icons/gauge';
	import MoonIcon from '@lucide/svelte/icons/moon';
	import PauseIcon from '@lucide/svelte/icons/pause';
	import PlayIcon from '@lucide/svelte/icons/play';
	import RotateCcwIcon from '@lucide/svelte/icons/rotate-ccw';
	import RotateCwIcon from '@lucide/svelte/icons/rotate-cw';
	import SkipBackIcon from '@lucide/svelte/icons/skip-back';
	import SkipForwardIcon from '@lucide/svelte/icons/skip-forward';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import * as Select from '@stump/ui/components/ui/select';
	import type { ReaderBookQuery } from '$lib/graphql/generated/graphql';

	type ReaderAudio = NonNullable<NonNullable<ReaderBookQuery['mediaById']>['audio']>;
	type ReaderAudioTrack = ReaderAudio['tracks'][number];

	let {
		audio,
		startPositionMs = 0,
		onPosition
	}: {
		audio: ReaderAudio;
		startPositionMs?: number;
		onPosition: (payload: {
			positionMs: number;
			trackIndex: number;
			isComplete: boolean;
		}) => void;
	} = $props();

	const SKIP_BACK_MS = 15_000;
	const SKIP_FORWARD_MS = 30_000;
	/**
	 * Going back a chapter restarts the current one first, which is what
	 * every audiobook player does and the only way to re-hear the opening of
	 * a chapter you are deep into without reaching for the list.
	 */
	const CHAPTER_RESTART_MS = 3_000;
	/**
	 * How far the playhead has to move before the position is handed to the
	 * parent's debounced writer again. Playback emits `timeupdate` about four
	 * times a second and the parent's debounce restarts on every call, so
	 * reporting each tick would keep the timer permanently re-armed and
	 * nothing would ever be written until the book was paused. Measuring the
	 * gap in *publication* time also means a stall reports nothing at all,
	 * because a stalled element still ticks without moving.
	 */
	const REPORT_STEP_MS = 10_000;
	const RATES = [0.75, 1, 1.25, 1.5, 1.75, 2];
	const SLEEP_OPTIONS = [
		{ value: 'off', label: 'No sleep timer' },
		{ value: '5', label: 'In 5 minutes' },
		{ value: '15', label: 'In 15 minutes' },
		{ value: '30', label: 'In 30 minutes' },
		{ value: '45', label: 'In 45 minutes' },
		{ value: '60', label: 'In 60 minutes' },
		{ value: 'chapter', label: 'End of chapter' }
	];

	/**
	 * Split a publication offset into the file that holds it and the offset
	 * into that file: a publication offset belongs to the last track whose
	 * `startOffsetMs` is at or below it, which is the rule the server states
	 * on `MediaAudioTrack.startOffsetMs`.
	 */
	function locate(
		tracks: readonly ReaderAudioTrack[],
		positionMs: number
	): { index: number; offsetMs: number } {
		let index = 0;
		for (let candidate = 0; candidate < tracks.length; candidate += 1) {
			if (tracks[candidate].startOffsetMs > positionMs) break;
			index = candidate;
		}
		return { index, offsetMs: Math.max(0, positionMs - tracks[index].startOffsetMs) };
	}

	/** The inverse: the publication offset an element's clock reading means. */
	function publicationMs(track: ReaderAudioTrack, currentTime: number): number {
		return Math.round(track.startOffsetMs + currentTime * 1000);
	}

	/**
	 * `0:04:09` — clock time, which is how a listener reads a position.
	 * Truncated rather than rounded, so the readout never claims a second the
	 * playhead has not reached.
	 */
	function hms(ms: number): string {
		const total = Math.max(0, Math.floor(ms / 1000));
		const pad = (value: number) => String(value).padStart(2, '0');
		return `${Math.floor(total / 3600)}:${pad(Math.floor((total % 3600) / 60))}:${pad(total % 60)}`;
	}

	function clamp(value: number, min: number, max: number): number {
		return Math.min(Math.max(value, min), max);
	}

	// The parent only mounts this player once the book is loaded, so the
	// resume head is an opening value rather than a tracked one: a background
	// refetch must not yank the playhead back to where the server last heard
	// about it.
	const opening = untrack(() => {
		const head = clamp(startPositionMs, 0, audio.durationMs);
		return { head, ...locate(audio.tracks, head) };
	});
	let positionMs = $state(opening.head);
	let trackIndex = $state(opening.index);
	/**
	 * User intent, deliberately not the element's own `paused` flag: swapping
	 * `src` at a track boundary runs the media load algorithm, which pauses
	 * the element and fires `pause` even though playback is meant to carry on
	 * into the next file.
	 */
	let playing = $state(false);
	let rate = $state(1);
	let element = $state<HTMLAudioElement | null>(null);
	let failed = $state(false);
	/** Non-null while the seek bar is dragged: the pending publication offset,
	 * which wins over the element's clock so the thumb does not fight the
	 * playhead it is being dragged away from. */
	let scrubMs = $state<number | null>(null);
	let sleep = $state('off');
	/** Wall-clock expiry of a minute timer, and the clock it is compared to. */
	let sleepEndsAt = $state<number | null>(null);
	/**
	 * Publication offset an "end of chapter" timer stops at. Pinned when the
	 * timer is armed rather than tracked off the playhead: a target that
	 * moved to the next mark as each one was crossed would never be reached.
	 */
	let sleepAtPositionMs = $state<number | null>(null);
	let clock = $state(Date.now());

	/**
	 * The in-file offset a swapped-in track has to open at, applied once it
	 * reports metadata; `null` once there is nothing left to apply.
	 */
	let pendingOffsetMs: number | null = opening.offsetMs;
	/** True from a `src` swap until its `loadedmetadata`, so the `pause` the
	 * load algorithm fires is not mistaken for the listener pausing. */
	let swapping = false;
	let reportedPositionMs = opening.head;

	const track = $derived(audio.tracks[trackIndex]);
	const chapters = $derived(audio.chapters);
	const displayMs = $derived(scrubMs ?? positionMs);

	/**
	 * The chapter the playhead is in: the last mark at or before it. Marks
	 * that begin after zero leave a head-of-book gap, which belongs to the
	 * first chapter as far as a listener is concerned.
	 */
	const chapterIndex = $derived.by(() => {
		let index = 0;
		for (let candidate = 0; candidate < chapters.length; candidate += 1) {
			if (chapters[candidate].startMs > positionMs) break;
			index = candidate;
		}
		return index;
	});

	/**
	 * `endMs` is only set when the container states one, so a chapter
	 * otherwise runs to the next mark and the last one to the publication
	 * duration.
	 */
	const chapterEndMs = $derived(
		chapters[chapterIndex]?.endMs ?? chapters[chapterIndex + 1]?.startMs ?? audio.durationMs
	);

	const sleepRemainingMs = $derived.by(() => {
		if (sleep === 'off') return null;
		// End-of-chapter counts down in publication time, which is the unit
		// the chapter is measured in; a minute timer counts down in wall time.
		if (sleep === 'chapter') return Math.max(0, (sleepAtPositionMs ?? positionMs) - positionMs);
		return Math.max(0, (sleepEndsAt ?? clock) - clock);
	});

	const chapterNote = $derived(
		audio.chapterSource === 'PER_TRACK'
			? 'One per file — this publication ships no chapter marks, so Coppice synthesized these from the track list.'
			: null
	);

	// The first observation is the position the book opened at, which is
	// already what the server holds.
	let reportedChapter = untrack(() => chapterIndex);

	/**
	 * Hand the publication position to the parent's debounced writer. Guarded
	 * on an actual move: a stall or a re-seek to the same offset still emits
	 * `timeupdate`, and re-arming the debounce with an identical payload only
	 * delays the write that matters.
	 */
	function report(): void {
		if (positionMs === reportedPositionMs) return;
		reportedPositionMs = positionMs;
		onPosition({
			positionMs,
			trackIndex,
			isComplete: positionMs >= audio.durationMs
		});
	}

	/**
	 * Chapter boundaries are the coarse resume points a listener actually
	 * notices, so crossing one is written on its own rather than waiting for
	 * the next position step.
	 */
	$effect(() => {
		const index = chapterIndex;
		if (index === reportedChapter) return;
		reportedChapter = index;
		report();
	});

	// The interval exists only while a minute timer is armed, and its cleanup
	// is what clears the timer on unmount. A sleep timer is a bedtime rather
	// than a listening budget, so it keeps running while the book is paused.
	$effect(() => {
		if (sleep === 'off' || sleep === 'chapter') return;
		const interval = setInterval(() => {
			clock = Date.now();
			if (sleepEndsAt !== null && clock >= sleepEndsAt) sleepExpired();
		}, 1000);
		return () => clearInterval(interval);
	});

	function selectTrack(index: number, offsetMs: number): void {
		swapping = true;
		pendingOffsetMs = offsetMs;
		trackIndex = index;
	}

	/** Move the playhead to a publication offset, crossing files if needed. */
	function seek(targetMs: number): void {
		const target = clamp(Math.round(targetMs), 0, audio.durationMs);
		const { index, offsetMs } = locate(audio.tracks, target);
		positionMs = target;
		if (index !== trackIndex) {
			selectTrack(index, offsetMs);
		} else if (element) {
			element.currentTime = offsetMs / 1000;
		}
	}

	function stepChapter(delta: number): void {
		if (!chapters.length) return;
		if (delta < 0 && positionMs - chapters[chapterIndex].startMs > CHAPTER_RESTART_MS) {
			seek(chapters[chapterIndex].startMs);
			return;
		}
		seek(chapters[clamp(chapterIndex + delta, 0, chapters.length - 1)].startMs);
	}

	function togglePlay(): void {
		const media = element;
		if (!media) return;
		if (playing) {
			// `pause()` fires `pause`, which flips the flag and reports.
			media.pause();
			return;
		}
		playing = true;
		void media.play().catch(onPlayRejected);
	}

	function setRate(next: number): void {
		rate = next;
		if (element) element.playbackRate = next;
	}

	function setSleep(value: string): void {
		sleep = value;
		clock = Date.now();
		sleepAtPositionMs = value === 'chapter' ? chapterEndMs : null;
		sleepEndsAt = value === 'off' || value === 'chapter' ? null : clock + Number(value) * 60_000;
	}

	function sleepExpired(): void {
		sleep = 'off';
		sleepEndsAt = null;
		sleepAtPositionMs = null;
		const media = element;
		playing = false;
		// Pausing a running element fires `pause`, which flushes the position
		// on its own — and does it after the clock tick that comes with the
		// event, so it is the more accurate of the two. A timer that expires
		// while the book is already paused fires nothing, so that case is the
		// one that has to flush explicitly.
		if (media && !media.paused) media.pause();
		else report();
	}

	function onLoadedMetadata(): void {
		const media = element;
		if (!media) return;
		failed = false;
		// Every load resets `playbackRate` to `defaultPlaybackRate`, so the
		// chosen speed is re-applied per track instead of once per mount.
		media.playbackRate = rate;
		const offset = pendingOffsetMs;
		pendingOffsetMs = null;
		if (offset) media.currentTime = offset / 1000;
		swapping = false;
		if (playing) void media.play().catch(onPlayRejected);
	}

	function onPlayRejected(): void {
		// Autoplay policy: the first `play()` has to come from a gesture. The
		// intent flag has to follow the element or the button lies.
		playing = false;
	}

	function onTimeUpdate(): void {
		const media = element;
		// The load algorithm resets the clock to zero and fires `timeupdate`
		// for it, so a swapped-in track reads as the start of its own file
		// until the pending offset is applied. Trusting that would report —
		// and persist — a position the listener never went to.
		if (!media || swapping || scrubMs !== null) return;
		positionMs = publicationMs(track, media.currentTime);
		if (Math.abs(positionMs - reportedPositionMs) >= REPORT_STEP_MS) report();
		if (sleepAtPositionMs !== null && positionMs >= sleepAtPositionMs) sleepExpired();
	}

	function onPause(): void {
		// Two pauses are not the listener pausing: the media load algorithm
		// fires one on a `src` swap, and reaching the end of a file fires one
		// immediately *before* `ended`. On a folder book both happen
		// mid-publication, and swallowing them is what keeps `playing` true
		// across the boundary so the next track starts on its own.
		if (swapping || element?.ended) return;
		playing = false;
		report();
	}

	function onEnded(): void {
		if (trackIndex + 1 < audio.tracks.length) {
			// A folder audiobook is one publication: the next file continues
			// it, so a boundary is a `src` swap and not the end of playback.
			selectTrack(trackIndex + 1, 0);
			return;
		}
		positionMs = audio.durationMs;
		playing = false;
		report();
	}

	function onError(): void {
		failed = true;
		swapping = false;
		playing = false;
	}

	function onScrub(event: Event & { currentTarget: HTMLInputElement }): void {
		scrubMs = event.currentTarget.valueAsNumber;
	}

	function onScrubEnd(): void {
		const target = scrubMs;
		scrubMs = null;
		if (target !== null) seek(target);
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
		if (event.key === ' ') {
			// A focused button already treats Space as a click, so hijacking
			// it here would toggle playback twice.
			if (target instanceof HTMLElement && target.closest('button')) return;
			event.preventDefault();
			togglePlay();
		} else if (event.key === 'ArrowLeft') {
			event.preventDefault();
			seek(positionMs - SKIP_BACK_MS);
		} else if (event.key === 'ArrowRight') {
			event.preventDefault();
			seek(positionMs + SKIP_FORWARD_MS);
		}
	}

	// The parent's `onDestroy` flushes whatever is pending, and Svelte tears
	// children down before their parent, so the last position written is the
	// one the listener stopped at.
	onDestroy(report);
</script>

<svelte:window onkeydown={onKeydown} />

<audio
	bind:this={element}
	src={track.url}
	preload="metadata"
	onloadedmetadata={onLoadedMetadata}
	ontimeupdate={onTimeUpdate}
	onpause={onPause}
	onplay={() => (playing = true)}
	onended={onEnded}
	onerror={onError}
></audio>

<div class="flex flex-col gap-3">
	{#if failed}
		<Alert variant="destructive">
			<AlertTitle>Unable to play this track</AlertTitle>
			<AlertDescription>
				Track {trackIndex + 1} of {audio.tracks.length} could not be streamed. It may be missing
				from disk, or the browser may not decode {track.mime}.
			</AlertDescription>
		</Alert>
	{/if}

	<div class="flex flex-col gap-4 rounded-xl border bg-card p-4">
		<div class="flex flex-col gap-1.5">
			<input
				type="range"
				class="w-full accent-primary"
				min="0"
				max={audio.durationMs}
				step="1000"
				value={displayMs}
				aria-label="Seek"
				aria-valuetext={hms(displayMs)}
				oninput={onScrub}
				onchange={onScrubEnd}
			/>
			<div class="flex flex-wrap items-baseline gap-x-2 text-xs text-muted-foreground">
				<span class="tabular-nums">{hms(displayMs)} / {hms(audio.durationMs)}</span>
				<span class="ml-auto">
					{#if audio.tracks.length > 1}
						Track {trackIndex + 1} of {audio.tracks.length} ·
					{/if}
					{audio.codec.toUpperCase()}
				</span>
			</div>
		</div>

		<div class="flex flex-wrap items-center gap-2">
			<Button
				variant="outline"
				size="icon-sm"
				aria-label="Previous chapter"
				disabled={chapters.length === 0}
				onclick={() => stepChapter(-1)}
			>
				<SkipBackIcon />
			</Button>
			<Button
				variant="outline"
				size="sm"
				aria-label="Skip back 15 seconds"
				onclick={() => seek(positionMs - SKIP_BACK_MS)}
			>
				<RotateCcwIcon data-icon="inline-start" />
				15s
			</Button>
			<Button
				size="icon-lg"
				aria-label={playing ? 'Pause' : 'Play'}
				aria-pressed={playing}
				onclick={togglePlay}
			>
				{#if playing}
					<PauseIcon />
				{:else}
					<PlayIcon />
				{/if}
			</Button>
			<Button
				variant="outline"
				size="sm"
				aria-label="Skip forward 30 seconds"
				onclick={() => seek(positionMs + SKIP_FORWARD_MS)}
			>
				30s
				<RotateCwIcon data-icon="inline-end" />
			</Button>
			<Button
				variant="outline"
				size="icon-sm"
				aria-label="Next chapter"
				disabled={chapters.length === 0 || chapterIndex >= chapters.length - 1}
				onclick={() => stepChapter(1)}
			>
				<SkipForwardIcon />
			</Button>

			<div class="ml-auto flex flex-wrap items-center gap-2">
				{#if sleepRemainingMs !== null}
					<Badge variant="secondary" class="tabular-nums">
						Sleeping in {hms(sleepRemainingMs)}
					</Badge>
				{/if}
				<Select.Root
					type="single"
					value={String(rate)}
					onValueChange={(value) => setRate(Number(value))}
				>
					<Select.Trigger class="w-24" aria-label="Playback speed">
						<GaugeIcon data-icon="inline-start" />
						{rate}&times;
					</Select.Trigger>
					<Select.Content>
						{#each RATES as option (option)}
							<Select.Item value={String(option)} label={`${option}\u00d7`} />
						{/each}
					</Select.Content>
				</Select.Root>
				<Select.Root type="single" value={sleep} onValueChange={setSleep}>
					<Select.Trigger class="w-44" aria-label="Sleep timer">
						<MoonIcon data-icon="inline-start" />
						{SLEEP_OPTIONS.find((option) => option.value === sleep)?.label}
					</Select.Trigger>
					<Select.Content>
						{#each SLEEP_OPTIONS as option (option.value)}
							<Select.Item value={option.value} label={option.label} />
						{/each}
					</Select.Content>
				</Select.Root>
			</div>
		</div>
	</div>

	{#if chapters.length}
		<div class="flex flex-col gap-2 rounded-xl border bg-card p-4">
			<div class="flex flex-wrap items-baseline gap-x-2">
				<h2 class="text-sm font-semibold">Chapters</h2>
				{#if chapterNote}
					<span class="text-xs text-muted-foreground">{chapterNote}</span>
				{/if}
			</div>
			<ul class="flex max-h-[45vh] flex-col divide-y overflow-y-auto">
				{#each chapters as chapter, position (chapter.index)}
					{@const current = position === chapterIndex}
					<li>
						<button
							type="button"
							class="flex w-full items-baseline gap-3 rounded-md px-3 py-2 text-left text-sm hover:bg-muted aria-[current=true]:bg-muted aria-[current=true]:font-medium"
							aria-current={current ? 'true' : undefined}
							onclick={() => seek(chapter.startMs)}
						>
							<span class="truncate">{chapter.title ?? `Chapter ${chapter.index + 1}`}</span>
							<span class="ml-auto shrink-0 tabular-nums text-xs text-muted-foreground">
								{hms(chapter.startMs)}
							</span>
						</button>
					</li>
				{/each}
			</ul>
		</div>
	{/if}

	<p class="text-xs text-muted-foreground">
		Space plays and pauses; the arrow keys skip back 15 and forward 30 seconds. Positions are
		saved against the whole publication, so a folder of tracks resumes like one book.
	</p>
</div>
