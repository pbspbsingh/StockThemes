const CHART_JS_URL = "https://cdn.jsdelivr.net/npm/chart.js@4.4.9/dist/chart.umd.min.js";
let chartJsPromise;

function loadChartJs() {
    if (window.Chart) return Promise.resolve(window.Chart);
    if (!chartJsPromise) {
        chartJsPromise = new Promise((resolve, reject) => {
            const script = document.createElement("script");
            script.src = CHART_JS_URL;
            script.onload = () => resolve(window.Chart);
            script.onerror = () => reject(new Error("Failed to load Chart.js"));
            document.head.appendChild(script);
        });
    }
    return chartJsPromise;
}

const naPlugin = {
    id: "fundamentalsNa",
    afterDatasetsDraw(chart) {
        const indices = chart.options.plugins.fundamentalsNa?.indices ?? [];
        const { ctx, scales } = chart;
        ctx.save();
        ctx.fillStyle = "#777";
        ctx.font = "10px sans-serif";
        ctx.textAlign = "center";
        for (const index of indices) ctx.fillText("N/A", scales.x.getPixelForValue(index), chart.chartArea.bottom - 8);
        ctx.restore();
    },
};

export class FundamentalsChart {
    constructor({ status, earnings, updated, refreshButton, canvases, summaries, isCurrent }) {
        this.status = status;
        this.earnings = earnings;
        this.updated = updated;
        this.refreshButton = refreshButton;
        this.canvases = canvases;
        this.summaries = summaries;
        this.isCurrent = isCurrent;
        this.charts = [];
        this.cache = new Map();
        this.requestId = 0;
    }

    render(info, { refresh = false } = {}) {
        const requestId = ++this.requestId;
        const key = this.key(info);
        if (!refresh && this.cache.has(key)) {
            const data = this.cache.get(key);
            this.draw(data);
            this.setUpdated(data);
            this.setStatus("");
            this.refreshButton.disabled = false;
            return;
        }
        if (!refresh) {
            this.destroyCharts();
            this.clearSummaries();
            this.earnings.textContent = "";
            this.updated.textContent = "";
        }
        this.setStatus(refresh ? "Refreshing fundamentals…" : "Loading fundamentals…");
        this.refreshButton.disabled = true;
        this.fetchAndDraw(info, requestId, refresh);
    }

    refresh(info) {
        this.render(info, { refresh: true });
    }

    cancel() {
        this.requestId++;
        this.refreshButton.disabled = false;
        this.destroyCharts();
    }

    async fetchAndDraw(info, requestId, refresh) {
        try {
            const path = `/api/fundamentals/${encodeURIComponent(info.exchange)}/${encodeURIComponent(info.ticker)}`;
            const response = await fetch(refresh ? `${path}/refresh` : path, {
                method: refresh ? "POST" : "GET",
            });
            if (!response.ok) throw new Error(await response.text());
            const [data] = await Promise.all([response.json(), loadChartJs()]);
            if (!this.isCurrentRequest(requestId, info)) return;
            this.cache.set(this.key(info), data);
            this.draw(data);
            this.setUpdated(data);
            this.setStatus("");
        } catch (error) {
            if (this.isCurrentRequest(requestId, info)) {
                this.setStatus(error.message || "Failed to load fundamentals", true);
            }
        } finally {
            if (this.isCurrentRequest(requestId, info)) this.refreshButton.disabled = false;
        }
    }

    isCurrentRequest(requestId, info) {
        return requestId === this.requestId && this.isCurrent(info);
    }

    key(info) {
        return `${info.exchange}:${info.ticker}`;
    }

    draw(data) {
        this.destroyCharts();
        const quarters = data.quarters.slice().reverse();
        const growth = field => {
            const historicalQuarters = quarters.slice(4);
            const historical = historicalQuarters.map((quarter, index) =>
                this.growthPercent(quarter[field], quarters[index]?.[field])
            );
            const forecastPrior = quarters.at(-4)?.[field];
            return {
                labels: [...historicalQuarters.map((quarter, index) =>
                    quarter.fiscal_period ?? `Quarter ${index + 1}`
                ), "Next Q"],
                values: historical,
                forecast: this.growthPercent(data.next_quarter[field], forecastPrior),
            };
        };

        const epsGrowth = growth("earnings_per_share");
        const revenueGrowth = growth("revenue");
        this.renderGrowthSummary(this.summaries.epsGrowth, "EPS YoY Growth", epsGrowth);
        this.renderGrowthSummary(this.summaries.revenueGrowth, "Revenue YoY Growth", revenueGrowth);
        this.renderEstimateSummary(
            this.summaries.epsEstimate,
            "EPS Surprise",
            quarters,
            "earnings_per_share",
            "earnings_per_share_estimate",
        );
        this.renderEstimateSummary(
            this.summaries.revenueEstimate,
            "Revenue Surprise",
            quarters,
            "revenue",
            "revenue_estimate",
        );

        this.charts.push(
            this.growthChart(this.canvases.epsGrowth, epsGrowth, "#4a9eff"),
            this.growthChart(this.canvases.revenueGrowth, revenueGrowth, "#f39c12"),
            this.estimateChart(
                this.canvases.epsEstimate,
                quarters,
                "earnings_per_share",
                "earnings_per_share_estimate",
                data.next_quarter.earnings_per_share,
                value => Number(value).toFixed(2),
            ),
            this.estimateChart(
                this.canvases.revenueEstimate,
                quarters,
                "revenue",
                "revenue_estimate",
                data.next_quarter.revenue,
                value => this.compact(value),
            ),
        );
    }

    growthPercent(current, prior) {
        return current == null || prior == null || prior === 0
            ? null
            : ((current - prior) / Math.abs(prior)) * 100;
    }

    renderGrowthSummary(element, label, data) {
        const values = data.values.map(value => this.valueSpan(value, value === null ? "N/A" : `${value.toFixed(1)}%`));
        const forecast = `<span class="forecast">${data.forecast === null ? "N/A" : `${data.forecast.toFixed(1)}% forecast`}</span>`;
        element.innerHTML = `<span class="summary-label">${label}:</span> ${[...values, forecast].join('<span class="separator">→</span>')}`;
    }

    renderEstimateSummary(element, label, quarters, actualField, estimateField) {
        const values = quarters.map(quarter => {
            const estimate = quarter[estimateField];
            const actual = quarter[actualField];
            const surprise = actual == null || estimate == null || estimate === 0
                ? null
                : ((actual - estimate) / Math.abs(estimate)) * 100;
            return this.valueSpan(surprise, surprise === null ? "N/A" : `${surprise.toFixed(1)}%`);
        });
        element.innerHTML = `<span class="summary-label">${label}:</span> ${values.join('<span class="separator">→</span>')}`;
    }

    valueSpan(value, label) {
        return `<span class="${this.valueClass(value)}">${label}</span>`;
    }

    valueClass(value) {
        if (value == null) return "na";
        return value >= 0 ? "positive" : "negative";
    }

    growthChart(canvas, data, color) {
        const naIndices = [...data.values, data.forecast]
            .map((value, index) => value === null ? index : null)
            .filter(index => index !== null);
        const forecastData = Array(data.values.length + 1).fill(null);
        if (data.values.length > 0) forecastData[data.values.length - 1] = data.values.at(-1);
        forecastData[data.values.length] = data.forecast;
        return new Chart(canvas, {
            type: "line",
            data: {
                labels: data.labels,
                datasets: [
                    {
                        label: "Historical",
                        data: [...data.values, null],
                        borderColor: color,
                        backgroundColor: color,
                        tension: 0.25,
                    },
                    {
                        label: "Forecast",
                        data: forecastData,
                        borderColor: color,
                        backgroundColor: "#1e1e1e",
                        borderDash: [5, 5],
                        pointBorderColor: color,
                        pointBorderWidth: 2,
                        tension: 0.25,
                    },
                ],
            },
            options: this.options(
                value => value === null ? "N/A" : `${value.toFixed(1)}%`,
                naIndices,
                () => "",
                context => context.dataset.label === "Forecast"
                    && context.dataIndex === data.values.length - 1,
            ),
            plugins: [naPlugin],
        });
    }

    estimateChart(canvas, quarters, actualField, estimateField, forecast, format) {
        const actual = quarters.map(quarter => quarter[actualField]);
        const estimates = quarters.map(quarter => quarter[estimateField]);
        const labels = quarters.map((quarter, index) => quarter.fiscal_period ?? `Quarter ${index + 1}`);
        return new Chart(canvas, {
            type: "bar",
            data: {
                labels: [...labels, "Next Q"],
                datasets: [
                    {
                        label: "Estimate / Forecast",
                        data: [...estimates, forecast],
                        backgroundColor: "#777",
                    },
                    {
                        label: "Actual",
                        data: [...actual, null],
                        backgroundColor: [
                            ...actual.map((value, index) => {
                                if (value == null || estimates[index] == null) return "#777";
                                return value >= estimates[index] ? "#27ae60" : "#e74c3c";
                            }),
                            "#4a9eff",
                        ],
                    },
                ],
            },
            options: this.options(value => format(value), [], items => {
                const index = items[0]?.dataIndex;
                if (index === undefined || index >= quarters.length) return "";
                if (actual[index] == null || estimates[index] == null) return "Surprise: N/A";
                const surprise = actual[index] - estimates[index];
                const percent = estimates[index] === 0 ? "N/A" : `${(surprise / Math.abs(estimates[index]) * 100).toFixed(1)}%`;
                return `Surprise: ${format(surprise)} (${percent})`;
            }),
        });
    }

    options(format, naIndices = [], tooltipFooter = () => "", suppressTooltipLabel = () => false) {
        return {
            responsive: true,
            maintainAspectRatio: false,
            animation: false,
            interaction: {
                mode: "index",
                intersect: false,
            },
            plugins: {
                legend: { labels: { color: "#aaa", boxWidth: 10, font: { size: 10 } } },
                tooltip: {
                    callbacks: {
                        label: context => suppressTooltipLabel(context)
                            ? null
                            : `${context.dataset.label}: ${context.raw == null ? "N/A" : format(context.raw)}`,
                        footer: tooltipFooter,
                    },
                },
                fundamentalsNa: { indices: naIndices },
            },
            scales: {
                x: { ticks: { color: "#888", font: { size: 10 } }, grid: { color: "#292929" } },
                y: { ticks: { color: "#888", callback: format, font: { size: 10 } }, grid: { color: "#333" } },
            },
        };
    }

    compact(value) {
        if (value == null) return "N/A";
        return Intl.NumberFormat("en", { notation: "compact", maximumFractionDigits: 1 }).format(value);
    }

    setUpdated(data) {
        const latestEarnings = data.quarters.find(quarter => quarter.earnings_release_date)?.earnings_release_date;
        this.earnings.textContent = latestEarnings
            ? `Last earnings ${new Date(latestEarnings).toLocaleDateString()}`
            : "";
        this.updated.textContent = `Updated ${new Date(data.last_updated).toLocaleString()}`;
    }

    setStatus(message, error = false) {
        this.status.textContent = message;
        this.status.classList.toggle("error", error);
        this.status.classList.toggle("loading", Boolean(message) && !error);
    }

    destroyCharts() {
        for (const chart of this.charts) chart.destroy();
        this.charts = [];
    }

    clearSummaries() {
        for (const summary of Object.values(this.summaries)) summary.textContent = "";
    }
}
