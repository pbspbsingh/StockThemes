const RRG_PARAMS = "timeframe=daily&tail=20&history=200&period_weeks=10";

export class RrgChart {
    constructor({ body, canvas, tickerToggle, isCurrent }) {
        this.body = body;
        this.canvas = canvas;
        this.tickerToggle = tickerToggle;
        this.isCurrent = isCurrent;
        this.animationFrame = null;
        this.requestId = 0;
        this.showTicker = localStorage.getItem("rrgShowTicker") !== "false";
    }

    render(info) {
        const requestId = ++this.requestId;
        this.cancelAnimation();
        this.tickerToggle.classList.toggle("active", this.showTicker);

        requestAnimationFrame(() => {
            if (!this.isCurrentRequest(requestId, info)) return;
            this.resizeCanvas();
            this.renderMessage("Loading…", "#555");
            this.fetchAndDraw(info, requestId);
        });
    }

    toggleTicker(info) {
        this.showTicker = !this.showTicker;
        localStorage.setItem("rrgShowTicker", this.showTicker);
        this.render(info);
    }

    cancel() {
        this.requestId++;
        this.cancelAnimation();
    }

    isCurrentRequest(requestId, info) {
        return requestId === this.requestId && this.isCurrent(info);
    }

    resizeCanvas() {
        const dpr = window.devicePixelRatio || 1;
        const width = this.body.clientWidth;
        const height = this.body.clientHeight;
        this.canvas.width = width * dpr;
        this.canvas.height = height * dpr;
        this.canvas.style.width = `${width}px`;
        this.canvas.style.height = `${height}px`;
        this.canvas.getContext("2d").scale(dpr, dpr);
    }

    renderMessage(message, color) {
        const dpr = window.devicePixelRatio || 1;
        const width = this.canvas.width / dpr;
        const height = this.canvas.height / dpr;
        const ctx = this.canvas.getContext("2d");
        ctx.fillStyle = "#1e1e1e";
        ctx.fillRect(0, 0, width, height);
        ctx.fillStyle = color;
        ctx.font = "13px sans-serif";
        ctx.textAlign = "center";
        ctx.textBaseline = "middle";
        ctx.fillText(message, width / 2, height / 2);
    }

    async fetchAndDraw(info, requestId) {
        const sameEtf = info.sectorEtf && info.sectorEtf === info.industryEtf;
        const etfs = [
            ...(sameEtf
                ? [{ etf: info.sectorEtf, name: `${info.sector} / ${info.industry}`, color: "#f39c12" }]
                : [
                    info.sectorEtf ? { etf: info.sectorEtf, name: info.sector, color: "#f39c12" } : null,
                    info.industryEtf ? { etf: info.industryEtf, name: info.industry, color: "#4a9eff" } : null,
                ].filter(Boolean)),
            { etf: info.ticker, name: info.ticker, color: "#c678dd", isTicker: true },
        ];

        let datasets;
        try {
            datasets = await Promise.all(etfs.map(async entry => {
                const response = await fetch(`/api/rrg/${entry.etf}?${RRG_PARAMS}`);
                if (!response.ok) throw new Error(response.status);
                return {
                    ...(await response.json()),
                    ...entry,
                    isTicker: entry.isTicker ?? false,
                };
            }));
        } catch (_) {
            if (this.isCurrentRequest(requestId, info)) {
                this.renderMessage("Failed to load RRG data", "#e74c3c");
            }
            return;
        }

        if (!this.isCurrentRequest(requestId, info)) return;
        const visible = this.showTicker ? datasets : datasets.filter(dataset => !dataset.isTicker);
        this.drawAnimated(visible);
    }

    drawAnimated(datasets, { animate = true } = {}) {
        this.cancelAnimation();
        const ctx = this.canvas.getContext("2d");
        const dpr = window.devicePixelRatio || 1;
        const width = this.canvas.width / dpr;
        const height = this.canvas.height / dpr;
        const pad = 50;
        const sequences = datasets.map(dataset => ({
            points: [
                ...(dataset.tail || []).map(point => ({ x: point.rs_ratio, y: point.rs_momentum })),
                { x: dataset.rs_ratio, y: dataset.rs_momentum },
            ],
            label: dataset.name,
            sublabel: dataset.etf !== dataset.name ? dataset.etf : null,
            isTicker: dataset.isTicker,
            color: dataset.color,
        }));

        const allPoints = [...sequences.flatMap(sequence => sequence.points), { x: 100, y: 100 }];
        const viewportPadding = 1.5;
        let minX = Math.min(...allPoints.map(point => point.x)) - viewportPadding;
        let maxX = Math.max(...allPoints.map(point => point.x)) + viewportPadding;
        let minY = Math.min(...allPoints.map(point => point.y)) - viewportPadding;
        let maxY = Math.max(...allPoints.map(point => point.y)) + viewportPadding;
        const xRange = maxX - minX;
        const yRange = maxY - minY;
        if (xRange > yRange) {
            const difference = xRange - yRange;
            minY -= difference / 2;
            maxY += difference / 2;
        } else {
            const difference = yRange - xRange;
            minX -= difference / 2;
            maxX += difference / 2;
        }

        const margins = { top: pad, right: pad, bottom: pad, left: pad };
        const plotWidth = width - margins.left - margins.right;
        const plotHeight = height - margins.top - margins.bottom;
        const toX = value => margins.left + (value - minX) / (maxX - minX) * plotWidth;
        const toY = value => margins.top + (maxY - value) / (maxY - minY) * plotHeight;
        const renderedSequences = sequences
            .slice()
            .sort((left, right) => Number(left.isTicker) - Number(right.isTicker))
            .map(sequence => ({
                points: sequence.points.map(point => [toX(point.x), toY(point.y)]),
                label: sequence.label,
                sublabel: sequence.sublabel,
                color: sequence.color,
                isTicker: sequence.isTicker,
            }));
        const centerX = toX(100);
        const centerY = toY(100);
        const duration = animate ? 750 : 0;
        const startTime = performance.now();
        const easeInOut = value =>
            value < 0.5 ? 2 * value * value : 1 - Math.pow(-2 * value + 2, 2) / 2;

        const drawFrame = now => {
            const raw = duration === 0 ? 1 : Math.min(1, (now - startTime) / duration);
            const progress = easeInOut(raw);
            ctx.clearRect(0, 0, width, height);
            ctx.fillStyle = "#1e1e1e";
            ctx.fillRect(0, 0, width, height);
            this.drawQuadrants(ctx, margins, plotWidth, plotHeight, centerX, centerY);
            this.drawAxes(ctx, margins, plotWidth, plotHeight, centerX, centerY, width, height);
            this.drawSequences(ctx, renderedSequences, raw, progress);

            if (raw < 1) {
                this.animationFrame = requestAnimationFrame(drawFrame);
            } else {
                this.animationFrame = null;
            }
        };
        this.animationFrame = requestAnimationFrame(drawFrame);
    }

    drawQuadrants(ctx, margins, plotWidth, plotHeight, centerX, centerY) {
        ctx.fillStyle = "rgba(23,162,184,0.07)";
        ctx.fillRect(margins.left, margins.top, centerX - margins.left, centerY - margins.top);
        ctx.fillStyle = "rgba(40,167,69,0.07)";
        ctx.fillRect(centerX, margins.top, margins.left + plotWidth - centerX, centerY - margins.top);
        ctx.fillStyle = "rgba(220,53,69,0.07)";
        ctx.fillRect(margins.left, centerY, centerX - margins.left, margins.top + plotHeight - centerY);
        ctx.fillStyle = "rgba(255,193,7,0.07)";
        ctx.fillRect(centerX, centerY, margins.left + plotWidth - centerX, margins.top + plotHeight - centerY);

        ctx.font = "600 10px sans-serif";
        ctx.globalAlpha = 0.5;
        ctx.textBaseline = "top";
        ctx.fillStyle = "rgba(23,162,184,0.9)";
        ctx.textAlign = "left";
        ctx.fillText("RECOVERING", margins.left + 6, margins.top + 6);
        ctx.fillStyle = "rgba(40,167,69,0.9)";
        ctx.textAlign = "right";
        ctx.fillText("LEADING", margins.left + plotWidth - 6, margins.top + 6);
        ctx.textBaseline = "bottom";
        ctx.fillStyle = "rgba(220,53,69,0.9)";
        ctx.textAlign = "left";
        ctx.fillText("LAGGING", margins.left + 6, margins.top + plotHeight - 6);
        ctx.fillStyle = "rgba(255,193,7,0.9)";
        ctx.textAlign = "right";
        ctx.fillText("WEAKENING", margins.left + plotWidth - 6, margins.top + plotHeight - 6);
        ctx.globalAlpha = 1;
    }

    drawAxes(ctx, margins, plotWidth, plotHeight, centerX, centerY, width, height) {
        ctx.strokeStyle = "#3a3a3a";
        ctx.lineWidth = 1;
        ctx.setLineDash([4, 4]);
        ctx.beginPath();
        ctx.moveTo(centerX, margins.top);
        ctx.lineTo(centerX, margins.top + plotHeight);
        ctx.moveTo(margins.left, centerY);
        ctx.lineTo(margins.left + plotWidth, centerY);
        ctx.stroke();
        ctx.setLineDash([]);
        ctx.font = "11px sans-serif";
        ctx.fillStyle = "#555";
        ctx.textBaseline = "bottom";
        ctx.textAlign = "center";
        ctx.fillText("RS-Ratio →", margins.left + plotWidth / 2, height - 2);
        ctx.save();
        ctx.translate(12, margins.top + plotHeight / 2);
        ctx.rotate(-Math.PI / 2);
        ctx.textBaseline = "top";
        ctx.fillText("RS-Momentum →", 0, 0);
        ctx.restore();
    }

    drawSequences(ctx, sequences, raw, progress) {
        for (const sequence of sequences) {
            const points = sequence.points;
            if (!points.length) continue;
            let [tipX, tipY] = points[0];
            if (points.length >= 2) {
                const pathProgress = progress * (points.length - 1);
                const segment = Math.min(Math.floor(pathProgress), points.length - 2);
                const fraction = pathProgress - segment;
                tipX = points[segment][0] + (points[segment + 1][0] - points[segment][0]) * fraction;
                tipY = points[segment][1] + (points[segment + 1][1] - points[segment][1]) * fraction;
                ctx.beginPath();
                ctx.strokeStyle = sequence.color;
                ctx.lineWidth = 1.5;
                ctx.globalAlpha = 0.65;
                ctx.moveTo(points[0][0], points[0][1]);
                for (let index = 1; index <= segment; index++) ctx.lineTo(points[index][0], points[index][1]);
                ctx.lineTo(tipX, tipY);
                ctx.stroke();
                ctx.globalAlpha = 1;
            }
            if (raw < 1) {
                ctx.beginPath();
                ctx.arc(tipX, tipY, 3, 0, Math.PI * 2);
                ctx.fillStyle = sequence.color;
                ctx.fill();
            } else {
                this.drawSequenceLabel(ctx, sequence);
            }
        }
    }

    drawSequenceLabel(ctx, sequence) {
        const points = sequence.points;
        const last = points[points.length - 1];
        const previous = points.length >= 2 ? points[points.length - 2] : last;
        if (points.length >= 2) {
            const angle = Math.atan2(last[1] - previous[1], last[0] - previous[0]);
            const length = sequence.isTicker ? 12 : 9;
            ctx.fillStyle = sequence.color;
            ctx.beginPath();
            ctx.moveTo(last[0], last[1]);
            ctx.lineTo(last[0] - length * Math.cos(angle - Math.PI / 6), last[1] - length * Math.sin(angle - Math.PI / 6));
            ctx.lineTo(last[0] - length * Math.cos(angle + Math.PI / 6), last[1] - length * Math.sin(angle + Math.PI / 6));
            ctx.closePath();
            ctx.fill();
        }
        ctx.textAlign = "left";
        ctx.textBaseline = "middle";
        const labelX = last[0] + 9;
        if (sequence.sublabel) {
            ctx.font = "bold 12px sans-serif";
            ctx.fillStyle = sequence.color;
            ctx.fillText(sequence.label, labelX, last[1] - 7);
            ctx.font = "10px sans-serif";
            ctx.fillStyle = "rgba(255,255,255,0.45)";
            ctx.fillText(sequence.sublabel, labelX, last[1] + 6);
        } else {
            ctx.font = "bold 12px sans-serif";
            ctx.fillStyle = sequence.color;
            ctx.fillText(sequence.label, labelX, last[1]);
        }
    }

    cancelAnimation() {
        if (this.animationFrame) {
            cancelAnimationFrame(this.animationFrame);
            this.animationFrame = null;
        }
    }
}
