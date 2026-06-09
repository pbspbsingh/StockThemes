import { Controller } from "https://unpkg.com/@hotwired/stimulus@3.2.2/dist/stimulus.js";

import { FundamentalsChart } from "../fundamentals_chart.js";
import { popupApi } from "../popup_api.js";

export default class extends Controller {
    static targets = [
        "backdrop",
        "title",
        "status",
        "earnings",
        "updated",
        "refresh",
        "epsGrowth",
        "revenueGrowth",
        "epsEstimate",
        "revenueEstimate",
        "epsGrowthSummary",
        "revenueGrowthSummary",
        "epsEstimateSummary",
        "revenueEstimateSummary",
    ];

    connect() {
        this.info = null;
        this.fundamentalsChart = new FundamentalsChart({
            status: this.statusTarget,
            earnings: this.earningsTarget,
            updated: this.updatedTarget,
            refreshButton: this.refreshTarget,
            canvases: {
                epsGrowth: this.epsGrowthTarget,
                revenueGrowth: this.revenueGrowthTarget,
                epsEstimate: this.epsEstimateTarget,
                revenueEstimate: this.revenueEstimateTarget,
            },
            summaries: {
                epsGrowth: this.epsGrowthSummaryTarget,
                revenueGrowth: this.revenueGrowthSummaryTarget,
                epsEstimate: this.epsEstimateSummaryTarget,
                revenueEstimate: this.revenueEstimateSummaryTarget,
            },
            isCurrent: info => this.isOpen() && this.info?.ticker === info.ticker,
        });
        popupApi.register(this);
    }

    disconnect() {
        this.close();
        popupApi.unregister(this);
    }

    open(info) {
        if (!info?.ticker || !info.exchange) return;

        this.info = info;
        const financialsUrl = `https://www.tradingview.com/symbols/${encodeURIComponent(info.exchange)}-${encodeURIComponent(info.ticker)}/financials-income-statement/?statements-period=FQ`;
        this.titleTarget.innerHTML =
            `<a href="${financialsUrl}" target="_blank" rel="noopener noreferrer">${this.escapeHtml(info.ticker)}</a>`;
        this.backdropTarget.classList.add("open");
        document.body.style.overflow = "hidden";
        this.fundamentalsChart.render(info);
    }

    isOpen() {
        return this.backdropTarget.classList.contains("open");
    }

    closeFromBackdrop(event) {
        if (event.target === this.backdropTarget) this.close();
    }

    closeOnEscape(event) {
        if (event.key === "Escape") this.close();
    }

    close() {
        if (!this.isOpen()) return;

        this.backdropTarget.classList.remove("open");
        document.body.style.overflow = "";
        this.fundamentalsChart.cancel();
        this.info = null;
    }

    refreshFundamentals() {
        if (this.info) this.fundamentalsChart.refresh(this.info);
    }

    escapeHtml(value) {
        const node = document.createElement("div");
        node.textContent = String(value ?? "");
        return node.innerHTML;
    }
}
