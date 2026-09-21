import { MetadataProvider } from '@stump/graphql'

export const PROVIDER_LABELS: Partial<Record<MetadataProvider, string>> = {
	[MetadataProvider.Hardcover]: 'Hardcover',
	[MetadataProvider.ComicVine]: 'Comic Vine',
}

export const PROVIDERS = Object.values(MetadataProvider)
