import { hasArgs, pretty, stepTitle, summarizeStep } from "../lib/steps";

function CheckIcon() {
  return (
    <svg viewBox="0 0 20 20" aria-hidden="true">
      <path d="m5 10.5 3.2 3.2L15.5 6" />
    </svg>
  );
}

function ErrorIcon() {
  return (
    <svg viewBox="0 0 20 20" aria-hidden="true">
      <path d="m6 6 8 8M14 6l-8 8" />
    </svg>
  );
}

function inline(value, limit = 150) {
  let text;
  try {
    text = JSON.stringify(value);
  } catch {
    text = String(value);
  }
  if (!text) return "";
  return text.length > limit ? `${text.slice(0, limit)}…` : text;
}

function LogItem({ step }) {
  const failed = Boolean(step.is_error);
  return (
    <li className={`log-item ${failed ? "is-error" : ""}`}>
      <span className="log-rail">
        <span className="log-icon">{failed ? <ErrorIcon /> : <CheckIcon />}</span>
      </span>

      <div className="log-body">
        <div className="log-head">
          <span className="log-step">#{step.step}</span>
          <span className="log-title">{stepTitle(step)}</span>
          <code className="log-tool">{step.tool}</code>
          {step.auto ? <span className="log-tag">自动</span> : null}
          <span className={`log-result-flag ${failed ? "bad" : "good"}`}>
            is_error: {String(failed)}
          </span>
        </div>

        <p className="log-summary">{summarizeStep(step)}</p>

        {hasArgs(step) ? (
          <p className="log-args" title={inline(step.arguments, 800)}>
            <span>参数</span> <code>{inline(step.arguments)}</code>
          </p>
        ) : null}

        <details className="log-details">
          <summary>查看完整参数与结果</summary>
          <div className="log-detail-block">
            <div className="log-detail-label">arguments</div>
            <pre>{pretty(step.arguments)}</pre>
          </div>
          <div className="log-detail-block">
            <div className="log-detail-label">result</div>
            <pre>{pretty(step.result)}</pre>
          </div>
        </details>
      </div>
    </li>
  );
}

export default function LogTimeline({ logs = [] }) {
  if (!logs.length) return null;
  return (
    <ol className="log-timeline" aria-label="执行日志">
      {logs.map((step, index) => (
        <LogItem key={`${step.step}-${step.tool}-${index}`} step={step} />
      ))}
    </ol>
  );
}
