<script lang="ts">
	import CheckCircle2Icon from '@lucide/svelte/icons/check-circle-2';
	import CircleIcon from '@lucide/svelte/icons/circle';
	import DownloadIcon from '@lucide/svelte/icons/download';
	import FileCheck2Icon from '@lucide/svelte/icons/file-check-2';
	import SearchIcon from '@lucide/svelte/icons/search';
	import ShieldCheckIcon from '@lucide/svelte/icons/shield-check';
	import UploadCloudIcon from '@lucide/svelte/icons/upload-cloud';
	import { Progress } from '@stump/ui/components/ui/progress';
	import { statusDescription, statusLabel } from '$lib/requests';

	type Step = {
	key: string;
	label: string;
	icon: typeof SearchIcon;
};

	const steps: Step[] = [
		{ key: 'APPROVED', label: 'Approval', icon: ShieldCheckIcon },
		{ key: 'SEARCHING', label: 'Source search', icon: SearchIcon },
		{ key: 'RELEASE_SELECTED', label: 'Release', icon: FileCheck2Icon },
		{ key: 'GRABBING', label: 'Grab', icon: DownloadIcon },
		{ key: 'DOWNLOADING', label: 'Download', icon: DownloadIcon },
		{ key: 'IMPORTING', label: 'Import', icon: UploadCloudIcon },
		{ key: 'COMPLETED', label: 'Available', icon: CheckCircle2Icon }
	];

	const order: Record<string, number> = {
		PENDING: 0,
		AWAITING_APPROVAL: 0,
		APPROVED: 1,
		SEARCHING: 2,
		NEEDS_SELECTION: 3,
		RELEASE_SELECTED: 3,
		GRABBED: 4,
		GRABBING: 4,
		DOWNLOADING: 5,
		IMPORTING: 6,
		QUEUED: 6,
		COMPLETED: 7,
		FULFILLED: 7,
		FAILED: -1,
		REJECTED: -1,
		CANCELLED: -1,
		EXPIRED: -1
	};

	let {
		status,
		progress = null,
		message = null,
		error = null
	}: {
		status: string | null | undefined;
		progress?: number | null;
		message?: string | null;
		error?: string | null;
	} = $props();

	const normalizedStatus = $derived(status?.toUpperCase() ?? '');
	const activeIndex = $derived(order[normalizedStatus] ?? 0);
	const boundedProgress = $derived(
		progress === null || progress === undefined ? null : Math.max(0, Math.min(100, progress))
	);
	const failure = $derived(['FAILED', 'REJECTED', 'CANCELLED', 'EXPIRED'].includes(normalizedStatus));
</script>

<section class="rounded-xl border bg-card p-4" aria-labelledby="request-progress-heading">
	<div class="flex flex-wrap items-start justify-between gap-2">
		<div>
			<h2 id="request-progress-heading" class="font-medium">{statusLabel(status)}</h2>
			<p class="mt-1 text-sm text-muted-foreground">{message || statusDescription(status)}</p>
		</div>
		{#if boundedProgress !== null}
			<span class="text-sm tabular-nums text-muted-foreground">{Math.round(boundedProgress)}%</span>
		{/if}
	</div>

	{#if boundedProgress !== null && !failure}
		<Progress value={boundedProgress} class="mt-4" aria-label={`Request progress: ${Math.round(boundedProgress)} percent`} />
	{/if}

	<ol class="mt-5 grid grid-cols-2 gap-3 sm:grid-cols-4 lg:grid-cols-7">
		{#each steps as step, index (step.key)}
			{@const StepIcon = step.icon}
			<li class="flex items-center gap-2 text-xs {index <= activeIndex && !failure ? 'text-foreground' : 'text-muted-foreground'}">
				{#if index < activeIndex && !failure}
					<CheckCircle2Icon class="size-4 shrink-0 text-emerald-500" aria-hidden="true" />
				{:else if index === activeIndex && !failure}
					<StepIcon class="size-4 shrink-0 text-primary" aria-hidden="true" />
				{:else}
					<CircleIcon class="size-4 shrink-0" aria-hidden="true" />
				{/if}
				<span>{step.label}</span>
			</li>
		{/each}
	</ol>

	{#if error}
		<p class="mt-4 rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive" role="alert">
			{error}
		</p>
	{/if}
</section>
