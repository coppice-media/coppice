<script lang="ts">
	import { createMutation, useQueryClient } from '@tanstack/svelte-query';
	import { toast } from 'svelte-sonner';
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Button } from '@stump/ui/components/ui/button';
	import { Empty, EmptyDescription, EmptyHeader, EmptyTitle } from '@stump/ui/components/ui/empty';
	import { request } from '@stump/ui/graphql/client';
	import {
		RunIngestQualityFixDocument,
		type IngestItemQuery,
		type IngestMediaQualityReportQuery
	} from '$lib/graphql/generated/graphql';
	import { humanize } from '$lib/ingest/helpers';

	// A drop item's report and a library rework report are the same GraphQL
	// type read through two documents; both select `fix`, so the union reads
	// every field this panel renders.
	type DropItemReport = NonNullable<NonNullable<IngestItemQuery['ingestItem']>['qualityReport']>;
	type MediaReport = NonNullable<IngestMediaQualityReportQuery['ingestMediaQualityReport']>;
	type Report = DropItemReport | MediaReport;
	type Check = Report['checks'][number];

	let {
		report,
		dropItemId,
		disabled = false
	}: {
		report: Report | null;
		dropItemId: string | null;
		disabled?: boolean;
	} = $props();

	const queryClient = useQueryClient();
	let runningCheckId = $state<string | null>(null);

	const fixMutation = createMutation(() => ({
		mutationFn: (checkId: string) =>
			request(RunIngestQualityFixDocument, {
				input: { dropItemId: dropItemId as string, checkId }
			}),
		onSuccess: () => {
			// A repair rewrites the staged file and re-runs analysis, so every
			// key the sheet's own drop-item mutations touch is now stale.
			void queryClient.invalidateQueries({ queryKey: ['rework-items'] });
			void queryClient.invalidateQueries({ queryKey: ['ingest-item', dropItemId] });
			void queryClient.invalidateQueries({ queryKey: ['drop-items'] });
			void queryClient.invalidateQueries({ queryKey: ['analysis-queue'] });
			toast.success('Repair tool ran; the item was analysed again.');
		},
		onError: (error) =>
			toast.error(error instanceof Error ? error.message : 'Unable to run the repair tool.'),
		onSettled: () => {
			runningCheckId = null;
		}
	}));

	// Named tools get the verb a librarian recognises; anything else falls back
	// to the server's own summary rather than a guessed label.
	const FIX_LABELS: Record<string, string> = {
		'audio-assemble': 'Assemble to M4B',
		'audio-chapters': 'Write chapters',
		'meta-edit': 'Fix metadata'
	};

	let emptyDescription = $derived(
		dropItemId
			? 'Requeue this item to run deterministic checks.'
			: 'Run quality checks from the Library screen to generate one.'
	);

	function fixLabel(fix: NonNullable<Check['fix']>): string {
		const known = FIX_LABELS[fix.tool];
		if (known) return known;
		return fix.summary.length > 40 ? `${fix.summary.slice(0, 39)}…` : fix.summary;
	}

	function fixableBy(check: Check): NonNullable<Check['fix']> | null {
		if (!dropItemId || !check.fix) return null;
		return check.status === 'FAIL' || check.status === 'WARN' ? check.fix : null;
	}

	function runFix(checkId: string): void {
		if (!dropItemId) return;
		runningCheckId = checkId;
		fixMutation.mutate(checkId);
	}

	function json(value: unknown): string {
		return JSON.stringify(value, null, 2);
	}
</script>

<section aria-labelledby="quality-heading" class="flex flex-col gap-3">
	<div><h2 id="quality-heading" class="text-lg font-semibold">Quality report</h2><p class="text-sm text-muted-foreground">Algorithm {report?.algorithmVersion ?? 'not available'} · score {report?.score ?? 0}/100</p></div>
	{#if report?.checks.length}
		<div class="flex flex-col gap-2">
			{#each report.checks as check (check.checkId)}
				{@const fix = fixableBy(check)}
				<div class="rounded-lg border p-3">
					<div class="flex flex-wrap items-center justify-between gap-2"><div class="font-medium">{check.label}</div><div class="flex items-center gap-2"><Badge variant={check.status === 'FAIL' ? 'destructive' : check.status === 'WARN' ? 'secondary' : 'outline'}>{humanize(check.status)}</Badge><span class="text-xs text-muted-foreground">weight {check.weight} · +{check.contribution.toFixed(1)}</span></div></div>
					<p class="mt-2 text-sm text-muted-foreground">Normalized score {check.normalizedScore.toFixed(2)}</p>
					{#if fix}
						<div class="mt-2 flex flex-wrap items-center gap-2">
							<Button
								size="sm"
								disabled={disabled || fixMutation.isPending}
								onclick={() => runFix(check.checkId)}
							>
								{runningCheckId === check.checkId && fixMutation.isPending
									? 'Running…'
									: fixLabel(fix)}
							</Button>
							<span class="text-xs text-muted-foreground">{fix.summary}</span>
						</div>
					{/if}
					<pre class="mt-2 max-h-32 overflow-auto rounded bg-muted p-2 text-xs">{json(check.evidence)}</pre>
				</div>
			{/each}
		</div>
	{:else}
		<Empty><EmptyHeader><EmptyTitle>No quality report</EmptyTitle><EmptyDescription>{emptyDescription}</EmptyDescription></EmptyHeader></Empty>
	{/if}
</section>
