"""Offline spectral hypotheses: two-basis coordinates and unknown-thickness matching.

No specimen label/thickness is an input. Detector responses are explicit assumptions;
the output is a candidate ranking, never a measured material identification.
"""
from __future__ import annotations

import csv
import hashlib
import json
from pathlib import Path

import numpy as np
from scipy.spatial import cKDTree


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def edge_interpolate(energy: np.ndarray, values: np.ndarray, query: np.ndarray) -> np.ndarray:
    """Log-log interpolation; duplicate NIST rows retain below/above edge values."""
    if np.any(query < energy[0]) or np.any(query > energy[-1]):
        raise ValueError("coefficient extrapolation is forbidden")
    low = np.clip(np.searchsorted(energy, query, side="right") - 1, 0, len(energy) - 2)
    high = low + 1
    exact_last = query == energy[-1]
    fraction = np.divide(np.log(query / energy[low]), np.log(energy[high] / energy[low]),
                         out=np.zeros_like(query), where=energy[high] != energy[low])
    result = np.exp(np.log(values[low]) * (1 - fraction) + np.log(values[high]) * fraction)
    result[exact_last] = values[-1]
    return result


class Coefficients:
    def __init__(self, coefficient_dir: Path, elements: Path):
        self.root = coefficient_dir
        self.sources = {elements}
        with elements.open(encoding="utf-8-sig") as handle:
            self.mass = {r["符号"]: float(r["标准原子质量"]) for r in csv.DictReader(handle)}
        self.cache = {}

    def element(self, symbol: str, energy: np.ndarray, absorption=False) -> np.ndarray:
        if symbol not in self.cache:
            path = self.root / f"{symbol}.txt"
            self.sources.add(path)
            rows = []
            for line in path.read_text(encoding="utf-8").splitlines():
                try:
                    row = [float(x) for x in line.split()[:3]]
                except ValueError:
                    continue
                if len(row) == 3:
                    rows.append(row)
            self.cache[symbol] = np.array(rows)
        a = self.cache[symbol]
        return edge_interpolate(a[:, 0] * 1000, a[:, 2 if absorption else 1], energy)

    def compound(self, composition: dict, energy: np.ndarray, absorption=False) -> np.ndarray:
        masses = {s: float(n) * self.mass[s] for s, n in composition.items()}
        total = sum(masses.values())
        if total <= 0 or any(x < 0 for x in masses.values()):
            raise ValueError("invalid atomic-ratio composition")
        return sum(m / total * self.element(s, energy, absorption) for s, m in masses.items())


def local_curve(path: Path, energy: np.ndarray, keyword: str) -> np.ndarray:
    lines = [x for x in path.read_text(encoding="utf-8").splitlines() if x and not x.startswith("#")]
    header = lines[0].split("\t")
    column = next(i for i, x in enumerate(header) if keyword in x)
    rows = [x.split("\t") for x in lines[1:]]
    return edge_interpolate(np.array([float(x[0]) for x in rows]),
                            np.array([float(x[column]) for x in rows]), energy)


def detector_response(data: Path, energy: np.ndarray, counts: np.ndarray,
                      density_scale=1., mixing=0., filtration_mm_al=0., coefficients=None) -> np.ndarray:
    """Task4 geometry with Method glass curves as hypotheses, not its measured W."""
    weighted = counts * energy
    if filtration_mm_al:
        weighted *= np.exp(-coefficients.element("Al", energy) * 2.7 * filtration_mm_al / 10)
    layers = []
    preceding = np.ones_like(energy)
    for name, thickness_cm in [("green_glass", .1), ("blue_glass", .3)]:
        root = data / "mulit_layer_encoder"
        mu = local_curve(root / "attenuation" / f"{name}_attenuation_local.txt", energy, "Linear_mu_cm") * density_scale
        mu_en = local_curve(root / "energy_absorption" / f"{name}_energy_absorption_approx_local.txt", energy, "Linear_mu_en") * density_scale
        absorbed = -np.expm1(-mu * thickness_cm) * mu_en / mu
        layers.append(weighted * preceding * absorbed)
        preceding *= np.exp(-mu * thickness_cm)
    w = np.array(layers)
    w /= w.sum(axis=1, keepdims=True)
    # Fraction of each *open-normalized* camera response originating from the other layer.
    return np.array([[1 - mixing, mixing], [mixing, 1 - mixing]]) @ w


def forward(mu: np.ndarray, tau: np.ndarray, weights: np.ndarray) -> np.ndarray:
    """Beer-Lambert integration before -log; tau is areal density in g/cm2."""
    return -np.log(np.maximum(np.exp(-np.outer(np.asarray(tau), mu)) @ weights.T, 1e-30))


def basis_lookup(mu_basis: np.ndarray, weights: np.ndarray, ranges=(5., 3.), points=100):
    # Method's fixed Al-Cu basis. Positive coefficients are equivalent mass paths,
    # not proof the object consists of Al and Cu. Outside cone points are retained as failures.
    axes = [np.r_[0., np.geomspace(1e-5, maximum, points - 1)] for maximum in ranges]
    aa, bb = np.meshgrid(*axes, indexing="ij")
    coordinates = np.column_stack([aa.ravel(), bb.ravel()])
    response = np.empty((len(coordinates), 2))
    for start in range(0, len(coordinates), 400):
        attenuation = coordinates[start:start + 400] @ mu_basis
        response[start:start + 400] = -np.log(np.maximum(np.exp(-attenuation) @ weights.T, 1e-30))
    return coordinates, response


def invert_basis(observed: np.ndarray, coordinates: np.ndarray, response: np.ndarray, sigma: np.ndarray):
    _, index = cKDTree(response / sigma).query(observed / sigma)
    n = int(round(np.sqrt(len(coordinates))))
    if n * n != len(coordinates):
        raise ValueError("basis grid must be square")
    # Interpolate all four adjacent cells, including their constrained boundaries.
    # Nearest grid nodes alone overstate mismatch on a logarithmic thickness grid.
    best = np.full(len(observed), np.inf)
    fitted = np.zeros((len(observed), 2))
    target = observed / sigma
    for da, db in [(0, 0), (-1, 0), (0, -1), (-1, -1)]:
        ia = np.clip(index // n + da, 0, n - 2)
        ib = np.clip(index % n + db, 0, n - 2)
        corners = np.array([response[ia * n + ib], response[(ia + 1) * n + ib],
                            response[ia * n + ib + 1], response[(ia + 1) * n + ib + 1]]) / sigma
        u = np.full((len(observed), 2), .5)
        cross = corners[3] - corners[2] - corners[1] + corners[0]
        for _ in range(10):
            prediction = corners[0] + u[:, 0, None] * (corners[1] - corners[0]) + \
                u[:, 1, None] * (corners[2] - corners[0]) + u[:, 0, None] * u[:, 1, None] * cross
            ja = corners[1] - corners[0] + u[:, 1, None] * cross
            jb = corners[2] - corners[0] + u[:, 0, None] * cross
            delta = target - prediction
            det = ja[:, 0] * jb[:, 1] - ja[:, 1] * jb[:, 0]
            det = np.where(abs(det) > 1e-16, det, np.inf)
            step = np.column_stack([(delta[:, 0] * jb[:, 1] - delta[:, 1] * jb[:, 0]) / det,
                                    (ja[:, 0] * delta[:, 1] - ja[:, 1] * delta[:, 0]) / det])
            u = np.clip(u + step, 0, 1)
        # Include projections onto every boundary (Newton clipping is not an LS projection).
        trials = [(u[:, 0], u[:, 1])]
        for left, right, direction in [(0, 1, 0), (2, 3, 0), (0, 2, 1), (1, 3, 1)]:
            d = corners[right] - corners[left]
            t = np.clip(np.sum((target - corners[left]) * d, axis=1) / np.maximum((d * d).sum(axis=1), 1e-30), 0, 1)
            trials.append((t, np.full(len(t), float(left == 2))) if direction == 0
                          else (np.full(len(t), float(left == 1)), t))
        for ua, ub in trials:
            prediction = corners[0] + ua[:, None] * (corners[1] - corners[0]) + \
                ub[:, None] * (corners[2] - corners[0]) + ua[:, None] * ub[:, None] * cross
            error = np.sum((prediction - target) ** 2, axis=1)
            better = error < best
            best[better] = error[better]
            a0 = coordinates[ia * n + ib, 0]
            b0 = coordinates[ia * n + ib, 1]
            value = np.column_stack([a0 + ua * (coordinates[(ia + 1) * n + ib, 0] - a0),
                                     b0 + ub * (coordinates[ia * n + ib + 1, 1] - b0)])
            fitted[better] = value[better]
    return fitted, np.sqrt(best)


def candidate_trajectory(mu: np.ndarray, weights: np.ndarray, density: float, maximum_mm=40., points=600):
    tau = np.r_[0., np.geomspace(1e-6, density * maximum_mm / 10, points - 1)]
    return tau, forward(mu, tau, weights)


def match_unknown(observed: np.ndarray, candidates: list, curves: dict, weights: np.ndarray,
                  sigma: np.ndarray, maximum_mm=40.):
    """Profile out thickness for every material; no target-specific prior or labels."""
    scores, paths = [], []
    for candidate in candidates:
        tau, response = candidate_trajectory(curves[candidate["name"]], weights,
                                              candidate["density_g_cm3"], maximum_mm)
        _, index = cKDTree(response / sigma).query(observed / sigma)
        # Orthogonal projection onto adjacent segments removes grid-step bias.
        best = np.full(len(observed), np.inf)
        best_tau = np.zeros(len(observed))
        for offset in [-1, 0]:
            left = np.clip(index + offset, 0, len(tau) - 2)
            start = response[left] / sigma
            delta = (response[left + 1] - response[left]) / sigma
            u = np.clip(np.sum((observed / sigma - start) * delta, axis=1) /
                        np.maximum(np.sum(delta * delta, axis=1), 1e-30), 0, 1)
            error = np.sum((observed / sigma - start - u[:, None] * delta) ** 2, axis=1)
            better = error < best
            best[better] = error[better]
            best_tau[better] = (tau[left] + u * (tau[left + 1] - tau[left]))[better]
        scores.append(best)
        paths.append(best_tau)
    return np.array(scores).T, np.array(paths).T


def match_basis_unknown(observed_basis, basis_distance, candidates, curves, weights,
                        coordinates, response, sigma, basis_mu, maximum_mm=40.):
    """Match library trajectories after BOTH data and library enter Al-Cu coordinates.

    Exclude outside-cone responses. Thin-response covariance is only a diagnostic
    approximation; final quantitation uses independently measured basis covariance.
    """
    from scipy.spatial import cKDTree
    inverse = np.linalg.inv(weights @ basis_mu.T)
    basis_sigma = np.sqrt(np.diag(inverse @ np.diag(sigma ** 2) @ inverse.T))
    rows = []
    good = basis_distance <= 1
    for candidate in candidates:
        tau, p = candidate_trajectory(curves[candidate["name"]], weights,
                                      candidate["density_g_cm3"], maximum_mm)
        mapped, residual = invert_basis(p, coordinates, response, sigma)
        valid = (residual <= 1) & (np.max(p, axis=1) > .1)
        score, thickness = None, None
        if good.any() and valid.sum() >= 2:
            distance, index = cKDTree(mapped[valid] / basis_sigma).query(observed_basis[good] / basis_sigma)
            score = float(np.median(distance ** 2))
            thickness = float(np.median(tau[valid][index]) / candidate["density_g_cm3"] * 10)
        rows.append({"candidate": candidate["name"], "category": candidate["category"],
                     "medianSquaredBasisResidual": score, "medianEquivalentPathMm": thickness,
                     "validLibraryTrajectoryPoints": int(valid.sum())})
    rows.sort(key=lambda row: row["medianSquaredBasisResidual"] if row["medianSquaredBasisResidual"] is not None else np.inf)
    for rank, row in enumerate(rows):
        row["rank"] = rank + 1
    return rows, {"acceptedDataRays": int(good.sum()), "dataRays": len(good),
                  "approximateBasisNoiseSigma": basis_sigma.tolist(),
                  "rankingInterpretation": "diagnostic Al-Cu feature matching; thin-response covariance approximation, no material certification"}


def source_records(paths):
    return [{"path": str(p.resolve()), "sha256": sha256(p)} for p in sorted(set(paths))]
