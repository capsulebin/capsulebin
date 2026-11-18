/** Formats a byte count as a short human-readable size string. */
export function formatSize(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
}

/** Formats a Unix timestamp (seconds) as a short relative expiry string. */
export function formatExpiry(expiresAt: number | null): string {
    if (expiresAt === null) return 'No expiry';

    const seconds = expiresAt - Math.floor(Date.now() / 1000);
    if (seconds <= 0) return 'Expired';

    const minutes = Math.round(seconds / 60);
    if (minutes < 60) return `Expires in ${minutes} minute${minutes === 1 ? '' : 's'}`;

    const hours = Math.round(minutes / 60);
    if (hours < 24) return `Expires in ${hours} hour${hours === 1 ? '' : 's'}`;

    const days = Math.round(hours / 24);
    return `Expires in ${days} day${days === 1 ? '' : 's'}`;
}
