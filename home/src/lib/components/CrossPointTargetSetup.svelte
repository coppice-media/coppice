<script lang="ts">
	import { browser } from '$app/environment'
	import { createMutation, createQuery, useQueryClient } from '@tanstack/svelte-query'
	import CheckCircle2Icon from '@lucide/svelte/icons/check-circle-2'
	import CircleAlertIcon from '@lucide/svelte/icons/circle-alert'
	import LoaderCircleIcon from '@lucide/svelte/icons/loader-circle'
	import NetworkIcon from '@lucide/svelte/icons/network'
	import ShieldCheckIcon from '@lucide/svelte/icons/shield-check'
	import { toast } from 'svelte-sonner'
	import { Alert, AlertDescription, AlertTitle } from '@stump/ui/components/ui/alert'
	import { Badge } from '@stump/ui/components/ui/badge'
	import { Button } from '@stump/ui/components/ui/button'
	import { Checkbox } from '@stump/ui/components/ui/checkbox'
	import * as Card from '@stump/ui/components/ui/card'
	import * as Select from '@stump/ui/components/ui/select'
	import { Input } from '@stump/ui/components/ui/input'
	import { Label } from '@stump/ui/components/ui/label'
	import { request } from '@stump/ui/graphql/client'
	import { errorMessage } from '@stump/ui/utils/errors.js'
	import {
		CrosspointTargetDocument,
		UpdateCrosspointTargetDocument,
		VerifyCrosspointTargetDocument
	} from '$lib/graphql/generated/graphql'
	import type { CrosspointTargetFieldsFragment, CrosspointTransferProfileInput } from '$lib/graphql/generated/graphql'

	type CrosspointTransferProfile = CrosspointTargetFieldsFragment['profile']

	let { deviceId }: { deviceId: string } = $props()

	const DEFAULT_PROFILE: CrosspointTransferProfile = {
		optimizerEnabled: false,
		targetModel: 'AUTO',
		jpegQuality: 85,
		grayscale: true,
		autoCrop: false,
		splitLargeParagraphs: true,
		removeFonts: true,
		chunkBytes: 2048,
		retryCount: 3,
		retryDelaySeconds: 2,
		timeoutSeconds: 30,
		maxUploadBytes: 536_870_912
	}
	type Verification = {
		verified: boolean
		model: string
		serial: string
		hostOrIp: string
		httpPort: number
		wsPort: number
	}

	const queryClient = useQueryClient()
	const targetQuery = createQuery(() => ({
		queryKey: ['crosspoint-target', deviceId],
		queryFn: () => request(CrosspointTargetDocument, { deviceId }),
		enabled: browser && !!deviceId
	}))
	const target = $derived(targetQuery.data?.crosspointTarget ?? null)
	let profile = $state<CrosspointTransferProfile>({ ...DEFAULT_PROFILE })

	$effect(() => {
		if (target?.profile) profile = { ...target.profile }
	})

	let host = $state('')
	let rootPath = $state('')
	let verification = $state<Verification | null>(null)
	let verifyError = $state<string | null>(null)
	let profileError = $state<string | null>(null)
	let confirmDevice = $state(false)

	const verify = createMutation(() => ({
		mutationFn: () => request(VerifyCrosspointTargetDocument, { deviceId, host: host.trim() }),
		onSuccess: (result) => {
			const candidate = result.verifyCrosspointTarget as unknown as Verification
			verification = candidate
			verifyError = candidate.verified ? null : 'The device did not verify.'
			confirmDevice = false
		},
		onError: (error) => {
			verification = null
			verifyError = errorMessage(error)
		}
	}))

	const save = createMutation(() => ({
		mutationFn: () => {
			if (!verification?.verified) throw new Error('Verify the CrossPoint target before saving it.')
			const profileInput: CrosspointTransferProfileInput = {
				optimizerEnabled: profile.optimizerEnabled,
				targetModel: profile.targetModel,
				jpegQuality: profile.jpegQuality,
				grayscale: profile.grayscale,
				autoCrop: profile.autoCrop,
				splitLargeParagraphs: profile.splitLargeParagraphs,
				removeFonts: profile.removeFonts,
				chunkBytes: profile.chunkBytes,
				retryCount: profile.retryCount,
				retryDelaySeconds: profile.retryDelaySeconds,
				timeoutSeconds: profile.timeoutSeconds,
				maxUploadBytes: profile.maxUploadBytes
			}
			return request(UpdateCrosspointTargetDocument, {
				deviceId,
				input: {
					hostOrIp: verification.hostOrIp,
					httpPort: verification.httpPort,
					wsPort: verification.wsPort,
					rootPath: normalizedRootPath,
					profile: profileInput
				}
			})
		},
		onSuccess: () => {
			verification = null
			confirmDevice = false
			void queryClient.invalidateQueries({ queryKey: ['crosspoint-target', deviceId] })
			void queryClient.invalidateQueries({ queryKey: ['crosspoint-deliveries', deviceId] })
			toast.success('CrossPoint target saved.')
		},
		onError: (error) => toast.error(errorMessage(error))
	}))

	function isPrivateIpv4(value: string): boolean {
		const octets = value.trim().split('.')
		if (octets.length !== 4 || octets.some((octet) => !/^\d{1,3}$/.test(octet))) return false
		const [first, second, third, fourth] = octets.map(Number)
		if ([first, second, third, fourth].some((octet) => octet < 0 || octet > 255)) return false
		if (first === 10 || (first === 172 && second >= 16 && second <= 31) || (first === 192 && second === 168)) {
			return !(first === 10 && second === 0 && third === 0 && fourth === 0)
		}
		return false
	}
	function isSafeRootPath(value: string): boolean {
		const path = value.trim() || '/'
		if (!path.startsWith('/') || path.includes('..') || path.includes('\\') || path.includes('//')) return false
		return path
			.split('/')
			.filter(Boolean)
			.every((part) => /^[A-Za-z0-9 _.-]+$/.test(part))
	}
	type NumericProfileKey =
		| 'jpegQuality'
		| 'chunkBytes'
		| 'retryCount'
		| 'retryDelaySeconds'
		| 'timeoutSeconds'
		| 'maxUploadBytes'

	function setProfileNumber(key: NumericProfileKey, value: string): void {
		const parsed = Number(value)
		profile = { ...profile, [key]: Number.isFinite(parsed) ? parsed : Number.NaN }
		profileError = null
	}

	function setProfileFlag(
		key: 'optimizerEnabled' | 'grayscale' | 'autoCrop' | 'splitLargeParagraphs' | 'removeFonts',
		value: boolean
	): void {
		profile = { ...profile, [key]: value }
		profileError = null
	}

	function profileValidationError(): string | null {
		if (!Number.isInteger(profile.jpegQuality) || profile.jpegQuality < 1 || profile.jpegQuality > 100) {
			return 'JPEG quality must be a whole number between 1 and 100.'
		}
		if (!Number.isInteger(profile.chunkBytes) || profile.chunkBytes < 1 || profile.chunkBytes > 2048) {
			return 'Chunk bytes must be a whole number between 1 and 2,048.'
		}
		if (!Number.isInteger(profile.retryCount) || profile.retryCount < 0 || profile.retryCount > 7) {
			return 'Additional retries must be a whole number between 0 and 7 (maximum 8 total attempts).'
		}
		if (!Number.isInteger(profile.retryDelaySeconds) || profile.retryDelaySeconds < 0 || profile.retryDelaySeconds > 86_400) {
			return 'Retry delay must be a whole number between 0 and 86,400 seconds.'
		}
		if (!Number.isInteger(profile.timeoutSeconds) || profile.timeoutSeconds < 5 || profile.timeoutSeconds > 600) {
			return 'Socket timeout must be a whole number between 5 and 600 seconds.'
		}
		if (!Number.isInteger(profile.maxUploadBytes) || profile.maxUploadBytes < 1 || profile.maxUploadBytes > 2_147_483_647) {
			return 'Maximum upload must be a whole number between 1 and 2,147,483,647 bytes.'
		}
		return null
	}

	function runVerify(event: SubmitEvent): void {
		event.preventDefault()
		verifyError = null
		profileError = null
		verification = null
		confirmDevice = false
		const invalidProfile = profileValidationError()
		if (invalidProfile) {
			profileError = invalidProfile
			return
		}
		if (!isPrivateIpv4(host)) {
			verifyError = 'Enter a private IPv4 address (10/8, 172.16/12, or 192.168/16). Hostnames, loopback, link-local, and public addresses are not accepted.'
			return
		}
		if (!isSafeRootPath(rootPath)) {
			verifyError = 'Use a rooted destination path with safe single-component names; parent traversal and backslashes are not accepted.'
			return
		}
		verify.mutate()
	}

	function runSave(): void {
		const invalidProfile = profileValidationError()
		if (invalidProfile) {
			profileError = invalidProfile
			return
		}
		save.mutate()
	}

	function profileSummary(value: CrosspointTransferProfile): string {
		return `${value.targetModel} · JPEG ${value.jpegQuality} · ${value.grayscale ? 'grayscale' : 'color'} · ${value.chunkBytes} B chunks · ${value.retryCount} retries (${value.retryCount + 1} attempts)`
	}

	const savedHttpPort = $derived(verification?.verified ? verification.httpPort : target?.httpPort ?? 80)
	const savedWsPort = $derived(verification?.verified ? verification.wsPort : target?.wsPort ?? 81)
	const canSave = $derived(Boolean(verification?.verified && confirmDevice))
	const normalizedRootPath = $derived(rootPath.trim() || target?.rootPath || '/')
</script>

<Card.Root size="sm" class="gap-0">
	<Card.Header class="gap-2">
		<div class="flex items-start gap-3">
			<span class="flex size-9 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
				<NetworkIcon class="size-4" aria-hidden="true" />
			</span>
			<div class="min-w-0">
				<Card.Title class="text-base">CrossPoint LAN target</Card.Title>
				<Card.Description>
					Verify the device before Home can queue a local transfer. Only a confirmed private IPv4 address is accepted.
				</Card.Description>
			</div>
		</div>
	</Card.Header>
	<Card.Content class="flex flex-col gap-4 pt-0">
		{#if target}
			<div class="flex flex-col gap-2 rounded-lg border bg-muted/20 p-3 text-sm" aria-label="Saved CrossPoint target">
				<div class="flex flex-wrap items-center gap-2">
					<CheckCircle2Icon class="size-4 text-primary" aria-hidden="true" />
					<span class="font-medium">Verified target</span>
					<Badge variant="secondary">Saved</Badge>
				</div>
				<dl class="grid gap-x-4 gap-y-1 text-xs sm:grid-cols-2">
					<div><dt class="text-muted-foreground">IPv4</dt><dd class="font-mono">{target.hostOrIp}</dd></div>
					<div><dt class="text-muted-foreground">Device fingerprint</dt><dd class="font-mono break-all">{target.fingerprint?.model ?? '—'} · {target.fingerprint?.serial ?? '—'}</dd></div>
					<div><dt class="text-muted-foreground">HTTP status</dt><dd>Port {target.httpPort}</dd></div>
					<div><dt class="text-muted-foreground">WebSocket transfer</dt><dd>Port {target.wsPort}</dd></div>
					<div><dt class="text-muted-foreground">Remote root</dt><dd class="font-mono">{target.rootPath}</dd></div>
					<div><dt class="text-muted-foreground">Profile digest</dt><dd class="font-mono break-all">{target.profileDigest}</dd></div>
					<div><dt class="text-muted-foreground">Profile</dt><dd class="truncate" title={profileSummary(target.profile)}>{profileSummary(target.profile)}</dd></div>
				</dl>
				<p class="text-xs text-muted-foreground">Saved from the last successful <code>/api/status</code> check. Re-verify after the CrossPoint receives a new address.</p>
			</div>
		{/if}

		<form class="flex flex-col gap-3" onsubmit={runVerify} novalidate aria-label="Verify CrossPoint target">
			<div class="flex flex-col gap-1.5">
				<Label for={`crosspoint-host-${deviceId}`}>Private IPv4 address</Label>
				<div class="flex flex-col gap-2 sm:flex-row">
					<Input
						id={`crosspoint-host-${deviceId}`}
						value={host || target?.hostOrIp || ''}
						oninput={(event) => {
							host = event.currentTarget.value
							verification = null
							confirmDevice = false
						}}
						inputmode="decimal"
						placeholder="192.168.1.42"
						autocomplete="off"
						aria-describedby={`crosspoint-host-help-${deviceId}`}
						class="font-mono sm:max-w-xs"
					/>
					<Button type="submit" variant="outline" disabled={verify.isPending}>
						{#if verify.isPending}<LoaderCircleIcon class="animate-spin" data-icon="inline-start" aria-hidden="true" />{/if}
						{verify.isPending ? 'Checking /api/status…' : 'Verify target'}
					</Button>
				</div>
				<p id={`crosspoint-host-help-${deviceId}`} class="text-xs text-muted-foreground">
					Home checks <code>/api/status</code> and confirms the model and serial. Ports are fixed by the CrossPoint protocol: HTTP {savedHttpPort}, WebSocket {savedWsPort}.
				</p>
			</div>
			<div class="flex flex-col gap-1.5">
				<Label for={`crosspoint-root-${deviceId}`}>Remote root path</Label>
				<Input
					id={`crosspoint-root-${deviceId}`}
					value={normalizedRootPath}
					oninput={(event) => {
						rootPath = event.currentTarget.value
						verification = null
						confirmDevice = false
					}}
					placeholder="/"
					autocomplete="off"
					aria-describedby={`crosspoint-root-help-${deviceId}`}
					class="font-mono sm:max-w-xs"
				/>
				<p id={`crosspoint-root-help-${deviceId}`} class="text-xs text-muted-foreground">
					Used as the bounded destination root for queued uploads. Use <code>/</code> or safe rooted components only; no parent traversal.
				</p>
			</div>

			<fieldset class="flex flex-col gap-3 rounded-lg border bg-muted/10 p-3">
				<legend class="px-1 text-sm font-medium">Transfer profile</legend>
				<p class="text-xs text-muted-foreground">
					These values are saved for this target and copied into each queued delivery. Defaults: optimizer off, AUTO model, JPEG 85, grayscale on, auto-crop off, split paragraphs on, remove fonts on, 2,048-byte chunks, 3 additional retries (4 maximum attempts), 2-second retry delay, 30-second timeout, and 512 MiB maximum upload.
				</p>
				<div class="grid gap-2 sm:grid-cols-2">
					<label class="flex items-center gap-2 text-xs">
						<Checkbox
							checked={profile.optimizerEnabled}
							onCheckedChange={(value) => setProfileFlag('optimizerEnabled', Boolean(value))}
						/>
						<span>Enable EPUB optimizer</span>
					</label>
					<div class="flex flex-col gap-1">
						<Label for={`crosspoint-model-${deviceId}`}>Target model</Label>
						<Select.Root
							type="single"
							value={profile.targetModel}
							onValueChange={(value) => {
								if (value === 'AUTO' || value === 'X3' || value === 'X4') {
									profile = { ...profile, targetModel: value }
									profileError = null
								}
							}}
						>
							<Select.Trigger id={`crosspoint-model-${deviceId}`} class="h-9">
								{profile.targetModel}
							</Select.Trigger>
							<Select.Content>
								<Select.Item value="AUTO" label="AUTO (resolve at verification)" />
								<Select.Item value="X3" label="X3" />
								<Select.Item value="X4" label="X4" />
							</Select.Content>
						</Select.Root>
					</div>
					<label class="flex items-center gap-2 text-xs">
						<Checkbox
							checked={profile.grayscale}
							onCheckedChange={(value) => setProfileFlag('grayscale', Boolean(value))}
						/>
						<span>Convert images to grayscale</span>
					</label>
					<label class="flex items-center gap-2 text-xs">
						<Checkbox
							checked={profile.autoCrop}
							onCheckedChange={(value) => setProfileFlag('autoCrop', Boolean(value))}
						/>
						<span>Auto-crop images</span>
					</label>
					<label class="flex items-center gap-2 text-xs">
						<Checkbox
							checked={profile.splitLargeParagraphs}
							onCheckedChange={(value) => setProfileFlag('splitLargeParagraphs', Boolean(value))}
						/>
						<span>Split large paragraphs</span>
					</label>
					<label class="flex items-center gap-2 text-xs">
						<Checkbox
							checked={profile.removeFonts}
							onCheckedChange={(value) => setProfileFlag('removeFonts', Boolean(value))}
						/>
						<span>Remove embedded fonts</span>
					</label>
				</div>
				<div class="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
					<label class="flex flex-col gap-1 text-xs">
						<span>JPEG quality (1–100; default 85)</span>
						<Input
							type="number"
							min="1"
							max="100"
							step="1"
							value={profile.jpegQuality}
							oninput={(event) => setProfileNumber('jpegQuality', event.currentTarget.value)}
							aria-invalid={profileError !== null}
						/>
					</label>
					<label class="flex flex-col gap-1 text-xs">
						<span>Chunk bytes (1–2,048; default 2,048)</span>
						<Input
							type="number"
							min="1"
							max="2048"
							step="1"
							value={profile.chunkBytes}
							oninput={(event) => setProfileNumber('chunkBytes', event.currentTarget.value)}
							aria-invalid={profileError !== null}
						/>
					</label>
					<label class="flex flex-col gap-1 text-xs">
						<span>Additional retries (0–7; default 3)</span>
						<Input
							type="number"
							min="0"
							max="7"
							step="1"
							value={profile.retryCount}
							oninput={(event) => setProfileNumber('retryCount', event.currentTarget.value)}
							aria-invalid={profileError !== null}
							aria-describedby={`crosspoint-retries-help-${deviceId}`}
						/>
						<span id={`crosspoint-retries-help-${deviceId}`} class="text-muted-foreground">Retries after the first attempt; 3 means 4 maximum attempts.</span>
					</label>
					<label class="flex flex-col gap-1 text-xs">
						<span>Retry delay seconds (0–86,400; default 2)</span>
						<Input
							type="number"
							min="0"
							max="86400"
							step="1"
							value={profile.retryDelaySeconds}
							oninput={(event) => setProfileNumber('retryDelaySeconds', event.currentTarget.value)}
							aria-invalid={profileError !== null}
						/>
					</label>
					<label class="flex flex-col gap-1 text-xs">
						<span>Socket timeout seconds (5–600; default 30)</span>
						<Input
							type="number"
							min="5"
							max="600"
							step="1"
							value={profile.timeoutSeconds}
							oninput={(event) => setProfileNumber('timeoutSeconds', event.currentTarget.value)}
							aria-invalid={profileError !== null}
						/>
					</label>
					<label class="flex flex-col gap-1 text-xs">
						<span>Maximum upload bytes (1–2,147,483,647)</span>
						<Input
							type="number"
							min="1"
							max="2147483647"
							step="1"
							value={profile.maxUploadBytes}
							oninput={(event) => setProfileNumber('maxUploadBytes', event.currentTarget.value)}
							aria-invalid={profileError !== null}
						/>
						<span class="text-muted-foreground">Default: 536,870,912 bytes (512 MiB).</span>
					</label>
				</div>
			</fieldset>

			{#if profileError}
				<Alert variant="destructive">
					<CircleAlertIcon data-icon="inline-start" aria-hidden="true" />
					<AlertTitle>Invalid transfer profile</AlertTitle>
					<AlertDescription>{profileError}</AlertDescription>
				</Alert>
			{:else if verifyError}
				<Alert variant="destructive">
					<CircleAlertIcon data-icon="inline-start" aria-hidden="true" />
					<AlertTitle>Target was not verified</AlertTitle>
					<AlertDescription>{verifyError}</AlertDescription>
				</Alert>
			{:else if verification?.verified}
				<Alert>
					<ShieldCheckIcon data-icon="inline-start" aria-hidden="true" />
					<AlertTitle>CrossPoint status confirmed</AlertTitle>
					<AlertDescription>
						{verification.model} · serial <code>{verification.serial}</code> · {verification.hostOrIp} · HTTP {verification.httpPort} / WebSocket {verification.wsPort}
					</AlertDescription>
				</Alert>
				<div class="flex items-start gap-3 rounded-lg border border-dashed p-3">
					<Checkbox id={`crosspoint-confirm-${deviceId}`} bind:checked={confirmDevice} class="mt-0.5" />
					<Label for={`crosspoint-confirm-${deviceId}`} class="text-sm leading-snug font-normal">
						I confirm this is my CrossPoint and its File Transfer / Calibre Wireless activity is active now. Save this exact verified target for queued delivery.
					</Label>
				</div>
				<Button type="button" class="w-fit" onclick={runSave} disabled={!canSave || save.isPending}>
					{save.isPending ? 'Saving target…' : 'Save verified target'}
				</Button>
			{/if}
		</form>

		<div class="flex flex-col gap-2 rounded-lg border border-dashed p-3 text-xs text-muted-foreground">
			<p class="font-medium text-foreground">Physical transfer boundary</p>
			<p>File Transfer / Calibre Wireless must be running on the CrossPoint while work is transferring. The pinned firmware does not use Coppice rich-sync at a custom URL; this LAN lane is not a physical rich-sync claim.</p>
			<p>Home never discovers arbitrary hosts, sends credentials to the device, overwrites an existing file, or deletes a remote file.</p>
		</div>

		<div class="rounded-lg border bg-muted/10 p-3 text-xs text-muted-foreground">
			<p class="font-medium text-foreground">Queue snapshot</p>
			<p class="mt-1">
				Each queued delivery stores the resolved profile, digest, source revision, and destination path. AUTO resolves to the verified X3/X4 device model before transfer.
			</p>
		</div>
	</Card.Content>
</Card.Root>
