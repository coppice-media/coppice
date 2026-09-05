<script lang="ts">
	import { resolve } from '$app/paths';
	import { page } from '$app/state';
	import { Button } from '@stump/ui/components/ui/button';
	import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@stump/ui/components/ui/card';
	import { Input } from '@stump/ui/components/ui/input';
	import { Label } from '@stump/ui/components/ui/label';

	let username = $state('');
	let password = $state('');
	let submitting = $state(false);
	let errorMessage = $state<string | null>(null);
	let returnTo = $derived(page.url.searchParams.get('returnTo') ?? resolve('/drop'));

	async function submit(event: SubmitEvent): Promise<void> {
		event.preventDefault();
		errorMessage = null;
		submitting = true;
		try {
			const response = await fetch('/api/v2/auth/login', {
				method: 'POST',
				headers: { 'content-type': 'application/json' },
				credentials: 'include',
				body: JSON.stringify({ username, password })
			});
			if (!response.ok) {
				let message = 'The username or password was not accepted.';
				try {
					const payload = (await response.json()) as { message?: string; error?: string };
					message = payload.message ?? payload.error ?? message;
				} catch {
					// Keep the user-facing authentication message when the server has no JSON body.
				}
				throw new Error(message);
			}
			const destination = returnTo.startsWith('/') ? returnTo : resolve('/drop');
			window.location.href = destination;
		} catch (error) {
			errorMessage = error instanceof Error ? error.message : 'Unable to sign in.';
			submitting = false;
		}
	}
</script>

<svelte:head>
	<title>Sign in · Stump ingest</title>
</svelte:head>

<div class="flex min-h-screen items-center justify-center bg-muted/30 px-4 py-12">
	<Card class="w-full max-w-md">
		<CardHeader>
			<CardTitle>Sign in to Stump</CardTitle>
			<CardDescription>Use your Stump account to review and commit staged books.</CardDescription>
		</CardHeader>
		<CardContent>
			<form class="flex flex-col gap-5" onsubmit={submit}>
				<div class="flex flex-col gap-2">
					<Label for="username">Username</Label>
					<Input id="username" autocomplete="username" bind:value={username} required />
				</div>
				<div class="flex flex-col gap-2">
					<Label for="password">Password</Label>
					<Input
						id="password"
						type="password"
						autocomplete="current-password"
						bind:value={password}
						required
					/>
				</div>
				{#if errorMessage}
					<p class="text-sm text-destructive" role="alert">{errorMessage}</p>
				{/if}
				<Button type="submit" disabled={submitting}>
					{submitting ? 'Signing in…' : 'Sign in'}
				</Button>
			</form>
		</CardContent>
	</Card>
</div>
