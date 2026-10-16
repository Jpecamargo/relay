import styles from "./LatencySparkline.module.css";

export interface LatencySparklineProps {
  /** RTT em ms, do mais antigo ao mais recente. */
  values: ReadonlyArray<number>;
  width?: number;
  height?: number;
}

export function LatencySparkline({ values, width = 96, height = 24 }: LatencySparklineProps) {
  if (values.length === 0) return null;
  const max = Math.max(...values, 1);
  const stepX = values.length > 1 ? width / (values.length - 1) : 0;
  const y = (v: number) => height - 2 - (v / max) * (height - 4);
  const points = values.map((v, i) => `${(i * stepX).toFixed(1)},${y(v).toFixed(1)}`).join(" ");
  const last = values[values.length - 1] ?? 0;
  return (
    <span className={styles.wrap}>
      <svg
        role="img"
        aria-label={`Latência, últimas ${values.length} medições, máximo ${Math.round(max)} ms`}
        width={width}
        height={height}
        viewBox={`0 0 ${width} ${height}`}
        className={styles.svg}
      >
        <polyline points={points} fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinejoin="round" />
        <circle cx={(values.length - 1) * stepX} cy={y(last)} r="2" fill="currentColor" />
      </svg>
      <span className={styles.value}>{Math.round(last)} ms</span>
    </span>
  );
}
