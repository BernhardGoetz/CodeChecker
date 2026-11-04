import type { RunReport, StepOutput, TestResult, TestStatus, VariationResult } from "../api";
import { DiffView } from "./DiffView";
import { VisibleWhitespace } from "./VisibleWhitespace";

const LABEL: Record<TestStatus, string> = {
  passed: "Bestanden",
  failed: "Falsche Ausgabe",
  timeout: "Zeitüberschreitung",
  runtimeError: "Laufzeitfehler",
  error: "Fehler",
  formatMismatch: "Ausgabeformat weicht von Vorlage ab",
  wrongLogic: "Nur für die gegebenen Werte richtig",
};

function Output({ title, text, truncated }: { title: string; text: string; truncated: boolean }) {
  if (!text) return null;
  return (
    <details open={title === "stderr"}>
      <summary>
        {title}
        {truncated && " (nach 64 KB abgeschnitten)"}
      </summary>
      {/* stdout: trailing spaces and the final line break made visible. */}
      <pre className="output">{title === "stdout" ? <VisibleWhitespace text={text} /> : text}</pre>
    </details>
  );
}

function StepOutputs({ out }: { out: StepOutput }) {
  return (
    <>
      <Output title="stdout" text={out.stdout} truncated={out.stdoutTruncated} />
      <Output title="stderr" text={out.stderr} truncated={out.stderrTruncated} />
    </>
  );
}

function TestRow({ t }: { t: TestResult }) {
  return (
    <li className={`result ${t.status}`}>
      <div className="result-head">
        <span className={`status ${t.status}`}>{LABEL[t.status]}</span>
        <strong className="result-name">{t.name}</strong>
        {t.output && <span className="muted small">{t.output.durationMs} ms</span>}
      </div>
      {t.message && <p className="small">{t.message}</p>}
      {t.onlyTrailingWhitespaceDiffers && (
        <p className="note warn small">Unterschied nur am Zeilenende (Leerzeichen/Zeilenumbruch)</p>
      )}
      <DiffView diff={t.diff} />
      {t.output && <StepOutputs out={t.output} />}
    </li>
  );
}

/** Result of the variation check: red block with findings, or a quiet note. */
function VariationBlock({
  variation,
  busy,
  onCancel,
}: {
  variation: VariationResult | null;
  busy: string | null;
  onCancel: () => void;
}) {
  if (busy !== null) {
    return (
      <div className="row small variation-busy">
        <p className="note busy">{busy}</p>
        <button onClick={onCancel}>Abbrechen</button>
      </div>
    );
  }
  if (!variation) return null;
  if (variation.state === "skipped") {
    return <p className="muted small">Variationstest übersprungen: {variation.note}</p>;
  }
  if (variation.state === "ok") {
    return <p className="note ok small">Variationstest bestanden: {variation.note}</p>;
  }
  return (
    <div className="result wrongLogic variation-failed" role="alert">
      <div className="result-head">
        <span className="status wrongLogic">
          Logikfehler: Code liefert nur für die gegebenen Werte das richtige Ergebnis
        </span>
      </div>
      {variation.note && <p className="small">{variation.note}</p>}
      <ul className="variation-findings">
        {variation.findings.map((f, i) => (
          <li key={i}>
            <strong>{f.description}</strong>
            <div className="variation-compare">
              <div>
                <span className="muted small">Erwartet (Musterlösung)</span>
                <pre className="output">{f.expected}</pre>
              </div>
              <div>
                <span className="muted small">Tatsächlich (dieser Code)</span>
                <pre className="output">{f.actual}</pre>
              </div>
            </div>
          </li>
        ))}
      </ul>
      {variation.explanation && (
        <div className="small">
          <span className="muted">Erklärung der KI (nur ein Hinweis, sie entscheidet nichts):</span>{" "}
          {variation.explanationQuote && <code>{variation.explanationQuote}</code>}
          {variation.explanationQuote && " – "}
          {variation.explanation}
        </div>
      )}
    </div>
  );
}

export function ResultsPanel({
  report,
  variation = null,
  variationBusy = null,
  onCancelVariation = () => undefined,
}: {
  report: RunReport;
  variation?: VariationResult | null;
  variationBusy?: string | null;
  onCancelVariation?: () => void;
}) {
  const allPassed = report.compileOk && report.passed === report.total && report.total > 0;
  return (
    <div className="results">
      {!report.compileOk ? (
        <div className="result failed">
          <div className="result-head">
            <span className="status failed">Kompilierfehler</span>
            {report.compile?.timedOut && <span>Zeitlimit beim Kompilieren überschritten.</span>}
          </div>
          {report.hint && <p className="note warn">{report.hint}</p>}
          {report.compile && <StepOutputs out={report.compile} />}
        </div>
      ) : (
        <>
          <p className={allPassed ? "summary ok" : "summary fail"}>
            {report.passed} von {report.total} Tests bestanden
          </p>
          <VariationBlock variation={variation} busy={variationBusy} onCancel={onCancelVariation} />
          <ol className="result-list">
            {report.tests.map((t) => (
              <TestRow key={t.index} t={t} />
            ))}
          </ol>
        </>
      )}
      <p className="muted small">Image: {report.image}</p>
    </div>
  );
}
