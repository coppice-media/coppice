<script lang="ts">
	import { Badge } from '@stump/ui/components/ui/badge';
	import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@stump/ui/components/ui/card';
	import { PageHeader } from '@stump/ui/components/ui/page-header';
	import { getHomeSession } from '$lib/session.svelte';
	import ConnectionsAnnotationExports from '$lib/components/ConnectionsAnnotationExports.svelte';
	import ConnectionsHardcover from '$lib/components/ConnectionsHardcover.svelte';
	import ConnectionsKindle from '$lib/components/ConnectionsKindle.svelte';
	import ConnectionsNotifications from '$lib/components/ConnectionsNotifications.svelte';
	import ConnectionsAdmin from '$lib/components/ConnectionsAdmin.svelte';

	const session = getHomeSession();
	const isOwner = $derived(Boolean(session.user?.isServerOwner));
</script>

<svelte:head>
	<title>Connections · Coppice</title>
	<meta
		name="description"
		content="Manage your Kindle destinations, provider connections, and annotation exports on this Coppice server."
	/>
</svelte:head>

<div class="flex flex-col gap-8">
	<PageHeader
		title="Connections"
		description="Personal destinations and integrations stay separate from owner-only server settings."
	/>

	<section aria-labelledby="personal-connections-heading" class="flex flex-col gap-5">
		<div class="flex flex-wrap items-baseline gap-3">
			<h2 id="personal-connections-heading" class="text-xl font-semibold tracking-tight">Personal</h2>
			<p class="text-sm text-muted-foreground">Only you can see and change these connections.</p>
		</div>

		<div class="grid gap-5 xl:grid-cols-2">
			<section id="kindle" aria-labelledby="kindle-heading" class="flex flex-col gap-4 scroll-mt-20">
				<h3 id="kindle-heading" class="sr-only">Kindle</h3>
				<ConnectionsKindle />
			</section>

			<section id="hardcover" aria-labelledby="hardcover-heading" class="flex flex-col gap-4 scroll-mt-20">
				<h3 id="hardcover-heading" class="sr-only">Hardcover</h3>
				<ConnectionsHardcover />
			</section>
		</div>

		<section id="notifications" aria-labelledby="notifications-heading" class="scroll-mt-20">
			<Card>
				<CardHeader>
					<div class="flex flex-wrap items-start gap-2">
						<div class="mr-auto">
							<CardTitle id="notifications-heading" class="text-base">Notifications</CardTitle>
							<CardDescription>Personal channels and event routing. This is separate from the server SMTP sender below.</CardDescription>
						</div>
						<Badge variant="secondary">Personal</Badge>
					</div>
				</CardHeader>
				<CardContent>
					<ConnectionsNotifications />
				</CardContent>
			</Card>
		</section>

		<section id="exports" aria-labelledby="exports-heading" class="scroll-mt-20">
			<Card>
				<CardHeader>
					<div class="flex flex-wrap items-start gap-2">
						<div class="mr-auto">
							<CardTitle id="exports-heading" class="text-base">Markdown / Git exports</CardTitle>
							<CardDescription>Choose a safe relative destination and a readable per-book filename. The server never accepts an arbitrary host path.</CardDescription>
						</div>
						<Badge variant="secondary">Personal</Badge>
					</div>
				</CardHeader>
				<CardContent class="flex flex-col gap-4">
					<div class="grid gap-3 rounded-lg border border-dashed bg-muted/30 p-3 text-xs text-muted-foreground sm:grid-cols-3">
						<div><p class="font-medium text-foreground">Preset</p><p class="mt-1">Obsidian is the default; choose another preset only when your vault needs it.</p></div>
						<div><p class="font-medium text-foreground">Filename</p><p class="mt-1"><code>{'{{author}} - {{title}}/annotations.md'}</code> keeps one readable file per book.</p></div>
						<div><p class="font-medium text-foreground">Destination</p><p class="mt-1">Enter a contained relative folder. Host paths are selected by the server owner, never by this form.</p></div>
					</div>
					<ConnectionsAnnotationExports />
				</CardContent>
			</Card>
		</section>
	</section>

	<section aria-labelledby="system-connections-heading" class="flex flex-col gap-5">
		<div class="flex flex-wrap items-baseline gap-3">
			<h2 id="system-connections-heading" class="text-xl font-semibold tracking-tight">Admin / System</h2>
			<p class="text-sm text-muted-foreground">The shared transport and mounted export boundary.</p>
		</div>
		{#if isOwner}
			<ConnectionsAdmin />
		{:else}
			<Card>
				<CardContent class="py-6 text-sm text-muted-foreground">
					SMTP sender settings and the mounted export root are visible only to the server owner. Your personal connections above are still available.
				</CardContent>
			</Card>
		{/if}
	</section>
</div>
