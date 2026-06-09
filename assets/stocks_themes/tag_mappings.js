class TagMappings {
    constructor(pageTickers) {
        this.pageTickers = pageTickers;
        this.tagsByTicker = new Map();
        this.refreshPromise = null;
    }

    tagsForTicker(ticker) {
        return this.tagsByTicker.get(ticker) || [];
    }

    refresh() {
        if (!this.refreshPromise) {
            this.refreshPromise = this.fetchMappings()
                .finally(() => {
                    this.refreshPromise = null;
                });
        }
        return this.refreshPromise;
    }

    async fetchMappings() {
        const [stockViews, tags] = await Promise.all([
            this.fetchJson("/api/stock-tags"),
            this.fetchJson("/api/tags"),
        ]);
        const tagCounts = new Map(tags.map(tag => [tag.name.toLowerCase(), tag.stock_count ?? 0]));
        const nextTagsByTicker = new Map();

        stockViews.forEach(stock => {
            if (!this.pageTickers.has(stock.ticker)) return;

            const tagNames = (stock.tags || [])
                .map(tag => tag.name)
                .filter(Boolean)
                .sort((a, b) => {
                    const countDiff = (tagCounts.get(b.toLowerCase()) ?? 0) - (tagCounts.get(a.toLowerCase()) ?? 0);
                    return countDiff || a.localeCompare(b);
                });

            nextTagsByTicker.set(stock.ticker, tagNames);
        });

        this.tagsByTicker = nextTagsByTicker;
    }

    async fetchJson(url) {
        const response = await fetch(url);
        if (response.ok) return response.json();

        let message = `HTTP ${response.status}`;
        try {
            const body = await response.json();
            if (body?.error) message = body.error;
        } catch (_) {
            // Keep the HTTP status when the response body is not JSON.
        }
        throw new Error(message);
    }
}

window.TagMappings = TagMappings;
