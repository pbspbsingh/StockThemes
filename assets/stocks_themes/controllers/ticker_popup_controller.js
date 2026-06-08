import { Controller } from "https://unpkg.com/@hotwired/stimulus@3.2.2/dist/stimulus.js";

import { popupApi } from "../popup_api.js";
import { FundamentalsChart } from "../fundamentals_chart.js";
import { PopupCharts } from "../popup_charts.js";
import { RrgChart } from "../rrg_chart.js";

export default class extends Controller {
    static targets = [
        "backdrop",
        "title",
        "chartsTab",
        "tagsTab",
        "fundamentalsTab",
        "rrgTab",
        "chartsPanel",
        "tagsPanel",
        "fundamentalsPanel",
        "rrgPanel",
        "tagsFrame",
        "rrgTickerToggle",
        "fundamentalsStatus",
        "fundamentalsEarnings",
        "fundamentalsUpdated",
        "fundamentalsRefresh",
        "epsGrowth",
        "revenueGrowth",
        "epsEstimate",
        "revenueEstimate",
        "epsGrowthSummary",
        "revenueGrowthSummary",
        "epsEstimateSummary",
        "revenueEstimateSummary",
        "sectorLabel",
        "industryLabel",
        "industryPanel",
        "sectorContainer",
        "industryContainer",
        "rrgBody",
        "rrgCanvas",
    ];

    connect() {
        this.info = null;
        this.activeTabName = "fundamentals";
        this.popupCharts = new PopupCharts({
            sectorLabel: this.sectorLabelTarget,
            industryLabel: this.industryLabelTarget,
            industryPanel: this.industryPanelTarget,
            sectorContainer: this.sectorContainerTarget,
            industryContainer: this.industryContainerTarget,
        });
        this.rrgChart = new RrgChart({
            body: this.rrgBodyTarget,
            canvas: this.rrgCanvasTarget,
            tickerToggle: this.rrgTickerToggleTarget,
            isCurrent: info =>
                this.isOpen() &&
                this.activeTab() === "rrg" &&
                this.info?.ticker === info.ticker,
        });
        this.fundamentalsChart = new FundamentalsChart({
            status: this.fundamentalsStatusTarget,
            earnings: this.fundamentalsEarningsTarget,
            updated: this.fundamentalsUpdatedTarget,
            refreshButton: this.fundamentalsRefreshTarget,
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
            isCurrent: info =>
                this.isOpen() &&
                this.activeTab() === "fundamentals" &&
                this.info?.ticker === info.ticker,
        });
        popupApi.register(this);
    }

    disconnect() {
        this.close();
        popupApi.unregister(this);
    }

    open(info, tab = "fundamentals") {
        if (!info) return;

        this.info = info;
        const financialsUrl = `https://www.tradingview.com/symbols/${encodeURIComponent(info.exchange)}-${encodeURIComponent(info.ticker)}/financials-income-statement/`;
        this.titleTarget.innerHTML =
            `<a href="${financialsUrl}" target="_blank" rel="noopener noreferrer" style="color:#4a9eff;font-weight:600">${this.escapeHtml(info.ticker)}</a>`;
        this.backdropTarget.classList.add("open");
        document.body.style.overflow = "hidden";
        this.switchTo(tab);
    }

    switchTab(event) {
        this.switchTo(event.currentTarget.dataset.tab);
    }

    switchTo(tab) {
        const nextTab = ["charts", "rrg", "tags", "fundamentals"].includes(tab) ? tab : "fundamentals";
        const isCharts = nextTab === "charts";
        const isTags = nextTab === "tags";
        const isFundamentals = nextTab === "fundamentals";
        const isRrg = nextTab === "rrg";
        this.activeTabName = nextTab;

        this.chartsTabTarget.classList.toggle("active", isCharts);
        this.tagsTabTarget.classList.toggle("active", isTags);
        this.fundamentalsTabTarget.classList.toggle("active", isFundamentals);
        this.rrgTabTarget.classList.toggle("active", isRrg);
        this.chartsPanelTarget.classList.toggle("active", isCharts);
        this.tagsPanelTarget.classList.toggle("active", isTags);
        this.fundamentalsPanelTarget.classList.toggle("active", isFundamentals);
        this.rrgPanelTarget.classList.toggle("active", isRrg);
        this.rrgTickerToggleTarget.style.display = isRrg ? "" : "none";

        if (!this.info) return;

        if (isRrg) {
            this.popupCharts.destroy();
            this.fundamentalsChart.cancel();
            this.rrgChart.render(this.info);
        } else if (isCharts) {
            this.rrgChart.cancel();
            this.fundamentalsChart.cancel();
            this.popupCharts.render(this.info);
        } else if (isFundamentals) {
            this.popupCharts.destroy();
            this.rrgChart.cancel();
            this.fundamentalsChart.render(this.info);
        } else {
            this.popupCharts.destroy();
            this.rrgChart.cancel();
            this.fundamentalsChart.cancel();
            this.renderTags(this.info);
        }
    }

    move(direction) {
        const tabs = ["charts", "rrg", "tags", "fundamentals"];
        const index = tabs.indexOf(this.activeTabName);
        this.switchTo(tabs[Math.max(0, Math.min(tabs.length - 1, index + direction))]);
    }

    renderTags(info) {
        const src = `/tags_mgmt.html?ticker=${encodeURIComponent(info.ticker)}`;
        if (this.tagsFrameTarget.getAttribute("src") !== src) {
            this.tagsFrameTarget.setAttribute("src", src);
        }
    }

    activeTab() {
        return this.activeTabName;
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
        this.popupCharts.destroy();
        this.rrgChart.cancel();
        this.fundamentalsChart.cancel();
        this.info = null;
    }

    toggleRrgTicker() {
        if (this.info) this.rrgChart.toggleTicker(this.info);
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
