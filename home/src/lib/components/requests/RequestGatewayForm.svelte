<script lang="ts">
	import CheckCircle2Icon from '@lucide/svelte/icons/check-circle-2';
	import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
	import EyeOffIcon from '@lucide/svelte/icons/eye-off';
	import KeyRoundIcon from '@lucide/svelte/icons/key-round';
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert';
	import { Button } from '@stump/ui/components/ui/button';
	import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@stump/ui/components/ui/card';
	import { Input } from '@stump/ui/components/ui/input';
	import { Label } from '@stump/ui/components/ui/label';
	import { Switch } from '@stump/ui/components/ui/switch';
	import { gatewayHealthLabel } from '$lib/requests';

	type GatewaySettings = {
		endpoint: string;
		enabled: boolean;
		requireApproval: boolean;
		automationEnabled: boolean;
		scoringFloor: number;
		verificationThreshold: number;
		maxRetries: number;
		handoffRoot?: string | null;
		hasToken: boolean;
		tokenRedacted: string;
	};

	let {
		settings = null,
		busy = false,
		error = null,
		onsave
	}: {
		settings: GatewaySettings | null;
		busy?: boolean;
		error?: string | null;
		onsave?: (input: {
			endpoint: string;
			token: string;
			enabled: boolean;
			requireApproval: boolean;
			automationEnabled: boolean;
			scoringFloor: number;
			verificationThreshold: number;
			maxRetries: number;
			handoffRoot?: string;
		}) => void;
	} = $props();

	let endpoint = $state('');
	let token = $state('');
	let enabled = $state(false);
	let requireApproval = $state(true);
	let automationEnabled = $state(false);
	let scoringFloor = $state(80);
	let verificationThreshold = $state(70);
	let maxRetries = $state(3);
	let handoffRoot = $state('');
	let touched = $state(false);

	$effect(() => {
		if (touched || !settings) return;
		endpoint = settings.endpoint;
		enabled = settings.enabled;
		requireApproval = settings.requireApproval;
		automationEnabled = settings.automationEnabled;
		scoringFloor = settings.scoringFloor;
		verificationThreshold = settings.verificationThreshold;
		maxRetries = settings.maxRetries;
		handoffRoot = settings.handoffRoot ?? '';
	});

	const configured = $derived(Boolean(settings?.endpoint && settings.hasToken));
	const healthStatus = $derived(
		!configured ? 'Not configured' : settings?.enabled ? 'Configured; health verified when a search runs' : 'Configured but paused'
	);
	const validEndpoint = $derived.by(() => {
		if (!endpoint.trim()) return false;
		try {
			const url = new URL(endpoint.trim());
			return (url.protocol === 'http:' || url.protocol === 'https:') && !url.username && !url.password && !url.search && !url.hash;
		} catch {
			return false;
		}
	});
	const validThresholds = $derived(
		Number.isInteger(scoringFloor) && scoringFloor >= 0 && scoringFloor <= 100 &&
			Number.isInteger(verificationThreshold) && verificationThreshold >= 0 && verificationThreshold <= 100 &&
			Number.isInteger(maxRetries) && maxRetries >= 0 && maxRetries <= 10
	);
	const canSave = $derived(validEndpoint && validThresholds && (Boolean(token.trim()) || Boolean(settings?.hasToken)));

	function submit(event: SubmitEvent): void {
		event.preventDefault();
		if (!canSave || busy) return;
		const nextToken = token.trim();
		if (!nextToken && !settings?.hasToken) return;
		onsave?.({
			endpoint: endpoint.trim(),
			token: nextToken,
			enabled,
			requireApproval,
			automationEnabled,
			scoringFloor,
			verificationThreshold,
			maxRetries,
			handoffRoot: handoffRoot.trim() || undefined
		});
		token = '';
		touched = false;
	}
</script>

<Card>
	<CardHeader>
		<div class="flex flex-wrap items-start gap-3">
			<div class="mr-auto">
				<CardTitle class="text-base">Private request gateway</CardTitle>
				<CardDescription class="mt-1 max-w-2xl">
					The gateway keeps tracker credentials and raw download URLs outside Coppice. This form stores only its private control endpoint and an encrypted gateway token.
				</CardDescription>
			</div>
			<div class="flex items-center gap-1.5 text-sm {configured ? 'text-emerald-500' : 'text-muted-foreground'}">
				{#if configured}
					<CheckCircle2Icon class="size-4" aria-hidden="true" />
				{:else}
					<CircleAlertIcon class="size-4" aria-hidden="true" />
				{/if}
				<span>{configured ? gatewayHealthLabel('OK') : gatewayHealthLabel('NOT_CONFIGURED')}</span>
			</div>
		</div>
	</CardHeader>
	<CardContent>
		{#if error}
			<Alert variant="destructive" class="mb-4">
				<AlertTitle>Could not save gateway settings</AlertTitle>
				<AlertDescription>{error}</AlertDescription>
			</Alert>
		{/if}
		<div class="mb-4 flex items-start gap-2 rounded-md border bg-muted/30 px-3 py-2.5 text-sm text-muted-foreground">
			<KeyRoundIcon class="mt-0.5 size-4 shrink-0" aria-hidden="true" />
			<p>
				Status: <strong class="font-medium text-foreground">{healthStatus}</strong>.
				Search and grab failures are shown on each request; credentials are never echoed here.
			</p>
		</div>

		<form class="grid gap-4" onsubmit={submit}>
			<div class="flex flex-col gap-2">
				<Label for="request-gateway-endpoint">Private gateway endpoint</Label>
				<Input
					id="request-gateway-endpoint"
					bind:value={endpoint}
					oninput={() => (touched = true)}
					type="url"
					placeholder="http://vpn-gateway:8787"
					autocomplete="url"
					required
				/>
				<p class="text-xs text-muted-foreground">Use only the internal control-network address. Tracker/MAM URLs and query strings are not accepted.</p>
			</div>
			<div class="flex flex-col gap-2">
				<Label for="request-gateway-token">Gateway token</Label>
				<Input
					id="request-gateway-token"
					bind:value={token}
					oninput={() => (touched = true)}
					type="password"
					placeholder={settings?.hasToken ? 'Leave blank to keep the current token' : 'Paste the gateway token'}
					autocomplete="new-password"
				/>
				<p class="flex items-center gap-1 text-xs text-muted-foreground">
					<EyeOffIcon class="size-3.5" aria-hidden="true" />
					{settings?.hasToken ? 'A token is stored; the secret is hidden.' : 'The secret is write-only and is never rendered after saving.'}
				</p>
			</div>

			<div class="grid gap-3 sm:grid-cols-3">
				<div class="flex flex-col gap-2">
					<Label for="request-score-floor">Minimum score</Label>
					<Input id="request-score-floor" type="number" min="0" max="100" step="1" bind:value={scoringFloor} oninput={() => (touched = true)} />
				</div>
				<div class="flex flex-col gap-2">
					<Label for="request-verification-threshold">Verification threshold</Label>
					<Input id="request-verification-threshold" type="number" min="0" max="100" step="1" bind:value={verificationThreshold} oninput={() => (touched = true)} />
				</div>
				<div class="flex flex-col gap-2">
					<Label for="request-max-retries">Maximum retries</Label>
					<Input id="request-max-retries" type="number" min="0" max="10" step="1" bind:value={maxRetries} oninput={() => (touched = true)} />
				</div>
			</div>
			<div class="flex flex-col gap-2">
				<Label for="request-handoff-root">Handoff root <span class="font-normal text-muted-foreground">(optional)</span></Label>
				<Input id="request-handoff-root" bind:value={handoffRoot} oninput={() => (touched = true)} placeholder="Configured server staging root" />
				<p class="text-xs text-muted-foreground">Files are hashed and staged before import; this is not a library path.</p>
			</div>

			<div class="grid gap-2 sm:grid-cols-3">
				<div class="flex items-center justify-between gap-3 rounded-md border px-3 py-2">
					<Label for="request-gateway-enabled" class="font-normal">Enable gateway</Label>
					<Switch id="request-gateway-enabled" bind:checked={enabled} onCheckedChange={() => (touched = true)} />
				</div>
				<div class="flex items-center justify-between gap-3 rounded-md border px-3 py-2">
					<Label for="request-require-approval" class="font-normal">Require approval</Label>
					<Switch id="request-require-approval" bind:checked={requireApproval} onCheckedChange={() => (touched = true)} />
				</div>
				<div class="flex items-center justify-between gap-3 rounded-md border px-3 py-2">
					<Label for="request-automation" class="font-normal">Automation (off by default)</Label>
					<Switch id="request-automation" bind:checked={automationEnabled} onCheckedChange={() => (touched = true)} />
				</div>
			</div>

			<div class="flex justify-end">
				<Button type="submit" disabled={!canSave || busy}>{busy ? 'Saving…' : 'Save gateway policy'}</Button>
			</div>
		</form>
	</CardContent>
</Card>
