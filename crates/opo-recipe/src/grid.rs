//! `grid`: a two-way layout on a surface (proposal 0001 §2 and §5, spec 0.2).
//!
//! The grid makes no geometry. It returns `{nodes, segments, cells, unused_overrides}` for
//! later steps to place members on with `for_each` and `instance`.

use std::collections::BTreeMap;

use opo_feel::{EPSILON, Value};

use crate::Error;

type V3 = [f64; 3];

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn mul(a: V3, k: f64) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn norm(a: V3) -> f64 {
    dot(a, a).sqrt()
}
fn unit(a: V3) -> V3 {
    let n = norm(a);
    if n < EPSILON { a } else { mul(a, 1.0 / n) }
}

/// One straight run of the surface: u along `dir` from `origin`, v along `up`.
#[derive(Debug, Clone)]
struct Run {
    origin: V3,
    dir: V3,
    up: V3,
    length: f64,
    normal: V3,
}

impl Run {
    fn at(&self, s: f64, t: f64) -> V3 {
        add(self.origin, add(mul(self.dir, s), mul(self.up, t)))
    }
}

/// A grid line along u (vertical members stand on it).
#[derive(Debug, Clone)]
struct ULine {
    cls: String,
    /// Run and position on it; a corner line is the end of the earlier run.
    run: usize,
    s: f64,
    normal: V3,
}

/// A grid line along v (a height).
#[derive(Debug, Clone)]
struct VLine {
    cls: String,
    t: f64,
}

/// A column (a bay along u).
#[derive(Debug, Clone)]
struct Column {
    run: usize,
    s0: f64,
    s1: f64,
    cls: String,
}

/// A row (a bay along v).
#[derive(Debug, Clone)]
struct Row {
    t0: f64,
    t1: f64,
    cls: String,
}

/// A member position between two nodes.
#[derive(Debug, Clone)]
pub struct Segment {
    /// `"h{j}.{column}"` or `"v{i}.{row}"`.
    pub id: String,
    /// `"u"` (horizontal on a wall: transoms) or `"v"` (vertical: mullions).
    pub direction: &'static str,
    /// Grid line index (j for u segments, i for v segments).
    pub line: usize,
    /// Column (u segments) or row (v segments).
    pub bay: usize,
    /// Class of its grid line.
    pub line_cls: String,
    /// Member class.
    pub cls: String,
    start: V3,
    end: V3,
    normal: V3,
    trim_start: f64,
    trim_end: f64,
    /// Cells on either side (ids, after merges).
    cells: Vec<String>,
    removed: bool,
    component: Value,
    values: Value,
}

/// A cell between grid lines; merged cells cover several.
#[derive(Debug, Clone)]
pub struct Cell {
    /// `"c{column}.{row}"` (lower-left of a merged cell).
    pub id: String,
    /// Column of its lower-left corner.
    pub column: usize,
    /// Row of its lower-left corner.
    pub row: usize,
    /// Columns and rows covered.
    columns: usize,
    rows: usize,
    /// Cell class.
    pub cls: String,
    /// Class of its column.
    pub column_cls: String,
    /// Class of its row.
    pub row_cls: String,
    component: Value,
    values: Value,
    merged_into: Option<usize>,
    panel: Option<([V3; 4], [f64; 4])>,
}

/// A grid before it becomes a FEEL value.
#[derive(Debug, Clone)]
pub struct Grid {
    runs: Vec<Run>,
    ulines: Vec<ULine>,
    vlines: Vec<VLine>,
    columns: Vec<Column>,
    rows: Vec<Row>,
    /// Member positions.
    pub segments: Vec<Segment>,
    /// Cells (merged ones are kept with `merged_into` set and left out of the result).
    pub cells: Vec<Cell>,
    unused: Vec<Value>,
}

fn op(msg: impl Into<String>) -> Error {
    Error::Op(format!("grid: {}", msg.into()))
}

fn num(v: &Value) -> Option<f64> {
    v.as_number()
}

fn point(v: &Value) -> Option<Vec<f64>> {
    match v {
        Value::List(l) => l.iter().map(num).collect(),
        _ => None,
    }
}

fn field<'a>(v: &'a Value, k: &str) -> Option<&'a Value> {
    match v {
        Value::Context(c) => c.get(k).filter(|x| !matches!(x, Value::Null)),
        _ => None,
    }
}

fn text(v: &Value, k: &str) -> Option<String> {
    match field(v, k) {
        Some(Value::String(s)) => Some(s.clone()),
        _ => None,
    }
}

fn v3(p: V3) -> Value {
    Value::List(p.iter().map(|x| Value::Number(*x)).collect())
}

fn ctx(pairs: Vec<(&str, Value)>) -> Value {
    Value::Context(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

fn s(x: &str) -> Value {
    Value::String(x.to_string())
}

/// The runs and v extent of a surface input.
fn runs_of(surface: &Value) -> Result<(Vec<Run>, f64), Error> {
    if let Some(face) = field(surface, "face") {
        let Value::List(pts) = face else {
            return Err(op("face must be a list of points"));
        };
        let p: Vec<V3> = pts
            .iter()
            .map(|q| match point(q).as_deref() {
                Some([x, y, z]) => Ok([*x, *y, *z]),
                _ => Err(op("face points are [x, y, z]")),
            })
            .collect::<Result<_, _>>()?;
        if p.len() != 4 {
            return Err(op(
                "a face has four corners (rectangular faces only in this version)",
            ));
        }
        let (u, v) = (sub(p[1], p[0]), sub(p[3], p[0]));
        if norm(sub(p[2], add(p[0], add(u, v)))) > 1e-6 || dot(unit(u), unit(v)).abs() > 1e-6 {
            return Err(op("the face must be a rectangle"));
        }
        let (du, dv) = (unit(u), unit(v));
        return Ok((
            vec![Run {
                origin: p[0],
                dir: du,
                up: dv,
                length: norm(u),
                normal: unit(cross(du, dv)),
            }],
            norm(v),
        ));
    }
    let baseline =
        field(surface, "baseline").ok_or_else(|| op("surface needs baseline or face"))?;
    let base = field(surface, "base").and_then(num).unwrap_or(0.0);
    let top = field(surface, "top")
        .and_then(num)
        .ok_or_else(|| op("surface needs top"))?;
    if top <= base + EPSILON {
        return Err(op("top must be above base"));
    }
    let Value::List(pts) = baseline else {
        return Err(op("baseline must be a list of points"));
    };
    let p: Vec<[f64; 2]> = pts
        .iter()
        .map(|q| match point(q).as_deref() {
            Some([x, y, ..]) => Ok([*x, *y]),
            _ => Err(op("baseline points are [x, y]")),
        })
        .collect::<Result<_, _>>()?;
    if p.len() < 2 {
        return Err(op("baseline needs at least two points"));
    }
    let mut runs = Vec::new();
    for w in p.windows(2) {
        let d = [w[1][0] - w[0][0], w[1][1] - w[0][1], 0.0];
        let len = norm(d);
        if len < EPSILON {
            return Err(op("baseline has a zero-length segment"));
        }
        let dir = unit(d);
        runs.push(Run {
            origin: [w[0][0], w[0][1], base],
            dir,
            up: [0.0, 0.0, 1.0],
            length: len,
            normal: [dir[1], -dir[0], 0.0],
        });
    }
    Ok((runs, top - base))
}

/// `(at, cls)` of each line and `(start, end, cls)` of each bay of a layout result.
#[allow(clippy::type_complexity)]
fn read_layout(v: &Value) -> Result<(Vec<(f64, String)>, Vec<(f64, f64, String)>), Error> {
    let list = |k: &str| match field(v, k) {
        Some(Value::List(l)) => Ok(l.clone()),
        _ => Err(op("the layout is invalid (no division fits)")),
    };
    let lines = list("lines")?
        .iter()
        .map(|l| {
            (
                field(l, "at").and_then(num).unwrap_or(0.0),
                text(l, "cls").unwrap_or_default(),
            )
        })
        .collect();
    let bays = list("bays")?
        .iter()
        .map(|b| {
            (
                field(b, "start").and_then(num).unwrap_or(0.0),
                field(b, "end").and_then(num).unwrap_or(0.0),
                text(b, "cls").unwrap_or_else(|| "main".into()),
            )
        })
        .collect();
    Ok((lines, bays))
}

impl Grid {
    /// Lay out `u` along each run of `surface` and `v` up it (both are `layout` argument
    /// contexts), with the default member and cell classes.
    pub fn build(surface: &Value, u: &Value, v: &Value) -> Result<Grid, Error> {
        let (runs, height) = runs_of(surface)?;
        let mut ulines: Vec<ULine> = Vec::new();
        let mut columns = Vec::new();
        let last = runs.len() - 1;
        for (r, run) in runs.iter().enumerate() {
            let (lines, bays) = read_layout(&opo_feel::layout(run.length, u))?;
            for (k, (at, cls)) in lines.iter().enumerate() {
                if k == 0 && r > 0 {
                    continue; // the corner line, already added as the end of the last run
                }
                let end_of_run = k == lines.len() - 1;
                let cls = if (k == 0 && r == 0) || (end_of_run && r == last) {
                    "border".to_string()
                } else if end_of_run {
                    "corner".to_string()
                } else {
                    cls.clone()
                };
                let normal = if end_of_run && r < last {
                    unit(add(run.normal, runs[r + 1].normal))
                } else {
                    run.normal
                };
                ulines.push(ULine {
                    cls,
                    run: r,
                    s: *at,
                    normal,
                });
            }
            for (s0, s1, cls) in bays {
                columns.push(Column {
                    run: r,
                    s0,
                    s1,
                    cls,
                });
            }
        }
        let (vl, vb) = read_layout(&opo_feel::layout(height, v))?;
        let vlines: Vec<VLine> = vl.into_iter().map(|(t, cls)| VLine { cls, t }).collect();
        let rows: Vec<Row> = vb
            .into_iter()
            .map(|(t0, t1, cls)| Row { t0, t1, cls })
            .collect();
        let mut g = Grid {
            runs,
            ulines,
            vlines,
            columns,
            rows,
            segments: Vec::new(),
            cells: Vec::new(),
            unused: Vec::new(),
        };
        g.topology();
        Ok(g)
    }

    fn node(&self, i: usize, j: usize) -> V3 {
        let l = &self.ulines[i];
        self.runs[l.run].at(l.s, self.vlines[j].t)
    }

    fn topology(&mut self) {
        let (nc, nr) = (self.columns.len(), self.rows.len());
        for (j, vl) in self.vlines.iter().enumerate() {
            for c in 0..nc {
                let cls = if vl.cls == "border" {
                    "boundary"
                } else {
                    "transom"
                };
                let mut cells = Vec::new();
                if j > 0 {
                    cells.push(format!("c{c}.{}", j - 1));
                }
                if j < nr {
                    cells.push(format!("c{c}.{j}"));
                }
                self.segments.push(Segment {
                    id: format!("h{j}.{c}"),
                    direction: "u",
                    line: j,
                    bay: c,
                    line_cls: vl.cls.clone(),
                    cls: cls.into(),
                    start: [0.0; 3],
                    end: [0.0; 3],
                    normal: self.runs[self.columns[c].run].normal,
                    trim_start: 0.0,
                    trim_end: 0.0,
                    cells,
                    removed: false,
                    component: Value::Null,
                    values: Value::Null,
                });
            }
        }
        for (i, ul) in self.ulines.iter().enumerate() {
            for r in 0..nr {
                let cls = match ul.cls.as_str() {
                    "border" => "boundary",
                    "corner" => "corner",
                    _ => "mullion",
                };
                let mut cells = Vec::new();
                if i > 0 {
                    cells.push(format!("c{}.{r}", i - 1));
                }
                if i < nc {
                    cells.push(format!("c{i}.{r}"));
                }
                self.segments.push(Segment {
                    id: format!("v{i}.{r}"),
                    direction: "v",
                    line: i,
                    bay: r,
                    line_cls: ul.cls.clone(),
                    cls: cls.into(),
                    start: [0.0; 3],
                    end: [0.0; 3],
                    normal: ul.normal,
                    trim_start: 0.0,
                    trim_end: 0.0,
                    cells,
                    removed: false,
                    component: Value::Null,
                    values: Value::Null,
                });
            }
        }
        let pts: Vec<(V3, V3)> = self
            .segments
            .iter()
            .map(|sg| match sg.direction {
                "u" => (self.node(sg.bay, sg.line), self.node(sg.bay + 1, sg.line)),
                _ => (self.node(sg.line, sg.bay), self.node(sg.line, sg.bay + 1)),
            })
            .collect();
        for (sg, (a, b)) in self.segments.iter_mut().zip(pts) {
            sg.start = a;
            sg.end = b;
        }
        for r in 0..nr {
            for c in 0..nc {
                let (cc, rc) = (self.columns[c].cls.clone(), self.rows[r].cls.clone());
                self.cells.push(Cell {
                    id: format!("c{c}.{r}"),
                    column: c,
                    row: r,
                    columns: 1,
                    rows: 1,
                    cls: if rc != "main" { rc.clone() } else { cc.clone() },
                    column_cls: cc,
                    row_cls: rc,
                    component: Value::Null,
                    values: Value::Null,
                    merged_into: None,
                    panel: None,
                });
            }
        }
    }

    /// A segment as the FEEL value later steps read.
    pub fn segment_value(&self, sg: &Segment) -> Value {
        ctx(vec![
            ("id", s(&sg.id)),
            ("direction", s(sg.direction)),
            ("line_cls", s(&sg.line_cls)),
            ("cls", s(&sg.cls)),
            ("start", v3(sg.start)),
            ("end", v3(sg.end)),
            ("path", Value::List(vec![v3(sg.start), v3(sg.end)])),
            ("length", Value::Number(norm(sub(sg.end, sg.start)))),
            ("normal", v3(sg.normal)),
            ("trim_start", Value::Number(sg.trim_start)),
            ("trim_end", Value::Number(sg.trim_end)),
            (
                "cells",
                Value::List(sg.cells.iter().map(|c| s(c)).collect()),
            ),
            ("component", sg.component.clone()),
            ("values", sg.values.clone()),
        ])
    }

    fn cell_box(&self, c: &Cell) -> (usize, f64, f64, f64, f64) {
        let (a, b) = (
            &self.columns[c.column],
            &self.columns[c.column + c.columns - 1],
        );
        (
            a.run,
            a.s0,
            b.s1,
            self.rows[c.row].t0,
            self.rows[c.row + c.rows - 1].t1,
        )
    }

    /// A cell as the FEEL value later steps read.
    pub fn cell_value(&self, c: &Cell) -> Value {
        let (r, s0, s1, t0, t1) = self.cell_box(c);
        let run = &self.runs[r];
        let outline = [
            run.at(s0, t0),
            run.at(s1, t0),
            run.at(s1, t1),
            run.at(s0, t1),
        ];
        let (panel, insets) = c.panel.unwrap_or((outline, [0.0; 4]));
        let ids = self.cell_segments(c);
        ctx(vec![
            ("id", s(&c.id)),
            ("column", Value::Number(c.column as f64)),
            ("row", Value::Number(c.row as f64)),
            ("cls", s(&c.cls)),
            ("column_cls", s(&c.column_cls)),
            ("row_cls", s(&c.row_cls)),
            (
                "outline",
                Value::List(outline.iter().map(|p| v3(*p)).collect()),
            ),
            (
                "panel_outline",
                Value::List(panel.iter().map(|p| v3(*p)).collect()),
            ),
            (
                "insets",
                Value::List(insets.iter().map(|x| Value::Number(*x)).collect()),
            ),
            ("width", Value::Number(s1 - s0)),
            ("height", Value::Number(t1 - t0)),
            ("origin", v3(outline[0])),
            ("x_axis", v3(run.dir)),
            ("y_axis", v3(run.up)),
            ("normal", v3(run.normal)),
            ("segments", Value::List(ids.iter().map(|i| s(i)).collect())),
            ("component", c.component.clone()),
            ("values", c.values.clone()),
        ])
    }

    /// Ids of the segments round a cell: bottom, right, top, left.
    fn cell_segments(&self, c: &Cell) -> Vec<String> {
        self.cell_sides(c).into_iter().flatten().collect()
    }

    fn cell_sides(&self, c: &Cell) -> [Vec<String>; 4] {
        let cols = c.column..c.column + c.columns;
        let rows = c.row..c.row + c.rows;
        [
            cols.clone().map(|k| format!("h{}.{k}", c.row)).collect(),
            rows.clone()
                .map(|k| format!("v{}.{k}", c.column + c.columns))
                .collect(),
            cols.map(|k| format!("h{}.{k}", c.row + c.rows)).collect(),
            rows.map(|k| format!("v{}.{k}", c.column)).collect(),
        ]
    }

    fn seg_index(&self, id: &str) -> Option<usize> {
        self.segments
            .iter()
            .position(|sg| sg.id == id && !sg.removed)
    }

    fn cell_index(&self, id: &str) -> Option<usize> {
        self.cells
            .iter()
            .position(|c| c.id == id && c.merged_into.is_none())
    }

    /// Apply instance overrides (proposal 0001 §5). Edits whose id is not in the grid, or that
    /// are invalid, are kept in `unused_overrides`.
    pub fn apply_overrides(&mut self, overrides: &Value) {
        let Value::List(edits) = overrides else {
            return;
        };
        for e in edits {
            let ok = if let Some(id) = text(e, "remove") {
                self.remove(&id)
            } else if let Some(id) = text(e, "segment") {
                match self.seg_index(&id) {
                    Some(k) => {
                        let sg = &mut self.segments[k];
                        if let Some(c) = text(e, "cls") {
                            sg.cls = c;
                        }
                        if let Some(c) = field(e, "component") {
                            sg.component = c.clone();
                        }
                        if let Some(v) = field(e, "values") {
                            sg.values = v.clone();
                        }
                        true
                    }
                    None => false,
                }
            } else if let Some(id) = text(e, "cell") {
                match self.cell_index(&id) {
                    Some(k) => {
                        let c = &mut self.cells[k];
                        if let Some(x) = text(e, "cls") {
                            c.cls = x;
                        }
                        if let Some(x) = field(e, "component") {
                            c.component = x.clone();
                        }
                        if let Some(v) = field(e, "values") {
                            c.values = v.clone();
                        }
                        true
                    }
                    None => false,
                }
            } else {
                false
            };
            if !ok {
                self.unused.push(e.clone());
            }
        }
    }

    /// Remove an inner segment and merge the cells on either side, if they form a rectangle.
    fn remove(&mut self, id: &str) -> bool {
        let Some(k) = self.seg_index(id) else {
            return false;
        };
        if matches!(self.segments[k].line_cls.as_str(), "border" | "corner") {
            return false;
        }
        let names = self.segments[k].cells.clone();
        let [a, b] = names.as_slice() else {
            return false;
        };
        let (Some(ia), Some(ib)) = (self.cell_index(a), self.cell_index(b)) else {
            return false;
        };
        let (ca, cb) = (self.cells[ia].clone(), self.cells[ib].clone());
        let horizontal = self.segments[k].direction == "u"; // merges a column of cells
        let fits = if horizontal {
            ca.column == cb.column && ca.columns == cb.columns
        } else {
            ca.row == cb.row && ca.rows == cb.rows
        };
        if !fits {
            return false;
        }
        let (lo, hi) = if (ca.column, ca.row) <= (cb.column, cb.row) {
            (ia, ib)
        } else {
            (ib, ia)
        };
        let other = self.cells[hi].clone();
        let keep = &mut self.cells[lo];
        if horizontal {
            keep.rows += other.rows;
        } else {
            keep.columns += other.columns;
        }
        let keep_id = keep.id.clone();
        self.cells[hi].merged_into = Some(lo);
        self.segments[k].removed = true;
        for sg in &mut self.segments {
            for c in &mut sg.cells {
                if *c == other.id {
                    *c = keep_id.clone();
                }
            }
        }
        true
    }

    /// Trims and panel outlines from the members' face widths (`widths`: class → width).
    pub fn finish(&mut self, join: &str, widths: &BTreeMap<String, f64>) -> Result<(), Error> {
        let continuous = match join {
            "u_continuous" => "v",
            "v_continuous" => "u",
            other => return Err(op(format!("unknown join '{other}'"))),
        };
        let w = |sg: &Segment| widths.get(&sg.cls).copied().unwrap_or(0.0);
        let by_id: BTreeMap<String, f64> = self
            .segments
            .iter()
            .filter(|sg| !sg.removed)
            .map(|sg| (sg.id.clone(), w(sg)))
            .collect();
        let width_of = |id: &str| by_id.get(id).copied();
        let mut trims = Vec::new();
        for sg in &self.segments {
            if sg.direction == continuous {
                trims.push((0.0, 0.0));
                continue;
            }
            // The continuous member through each end node: on the same line, on either side.
            let at = |node: usize| {
                let ids = if sg.direction == "u" {
                    [
                        format!("v{node}.{}", sg.line),
                        format!("v{node}.{}", sg.line.wrapping_sub(1)),
                    ]
                } else {
                    [
                        format!("h{node}.{}", sg.line),
                        format!("h{node}.{}", sg.line.wrapping_sub(1)),
                    ]
                };
                ids.iter().find_map(|i| width_of(i)).unwrap_or(0.0) / 2.0
            };
            trims.push((at(sg.bay), at(sg.bay + 1)));
        }
        for (sg, (a, b)) in self.segments.iter_mut().zip(trims) {
            sg.trim_start = a;
            sg.trim_end = b;
        }
        let mut panels = Vec::new();
        for c in &self.cells {
            if c.merged_into.is_some() {
                panels.push(None);
                continue;
            }
            let sides = self.cell_sides(c);
            let inset = |ids: &Vec<String>| {
                ids.iter()
                    .filter_map(|i| width_of(i))
                    .fold(0.0_f64, f64::max)
                    / 2.0
            };
            let ins = [
                inset(&sides[0]),
                inset(&sides[1]),
                inset(&sides[2]),
                inset(&sides[3]),
            ];
            let (r, s0, s1, t0, t1) = self.cell_box(c);
            let run = &self.runs[r];
            let (a0, a1, b0, b1) = (s0 + ins[3], s1 - ins[1], t0 + ins[0], t1 - ins[2]);
            panels.push(Some((
                [
                    run.at(a0, b0),
                    run.at(a1, b0),
                    run.at(a1, b1),
                    run.at(a0, b1),
                ],
                ins,
            )));
        }
        for (c, p) in self.cells.iter_mut().zip(panels) {
            c.panel = p;
        }
        Ok(())
    }

    /// The grid as a FEEL value.
    pub fn value(&self) -> Value {
        let mut nodes = Vec::new();
        for j in 0..self.vlines.len() {
            for i in 0..self.ulines.len() {
                nodes.push(ctx(vec![
                    ("id", s(&format!("n{i}.{j}"))),
                    ("point", v3(self.node(i, j))),
                    ("u_index", Value::Number(i as f64)),
                    ("v_index", Value::Number(j as f64)),
                ]));
            }
        }
        let segments = self
            .segments
            .iter()
            .filter(|sg| !sg.removed)
            .map(|sg| self.segment_value(sg))
            .collect();
        let cells = self
            .cells
            .iter()
            .filter(|c| c.merged_into.is_none())
            .map(|c| self.cell_value(c))
            .collect();
        ctx(vec![
            ("nodes", Value::List(nodes)),
            ("segments", Value::List(segments)),
            ("cells", Value::List(cells)),
            ("unused_overrides", Value::List(self.unused.clone())),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opo_feel::{Scope, eval_str};

    fn val(s: &str) -> Value {
        eval_str(s, &Scope::new()).unwrap()
    }

    fn grid(surface: &str, u: &str, v: &str) -> Grid {
        let mut g = Grid::build(&val(surface), &val(u), &val(v)).unwrap();
        g.finish("u_continuous", &BTreeMap::new()).unwrap();
        g
    }

    fn ids(v: &Value, list: &str) -> Vec<String> {
        match field(v, list) {
            Some(Value::List(l)) => l.iter().map(|x| text(x, "id").unwrap()).collect(),
            _ => panic!(),
        }
    }

    const WALL: &str = "{baseline: [[0, 0], [3, 0]], base: 0, top: 2}";
    const U3: &str = r#"{pattern: [{size: 3}], distribution: "count"}"#;

    #[test]
    fn straight_wall_counts_and_ids() {
        let g = grid(
            WALL,
            r#"{pattern: [{size: 3}], distribution: "count"}"#,
            r#"{pattern: [{size: 2}], distribution: "count"}"#,
        );
        let v = g.value();
        assert_eq!(ids(&v, "nodes").len(), 4 * 3);
        assert_eq!(
            ids(&v, "cells"),
            ["c0.0", "c1.0", "c2.0", "c0.1", "c1.1", "c2.1"]
        );
        let segs = ids(&v, "segments");
        assert_eq!(segs.len(), 3 * 3 + 4 * 2, "9 transoms and 8 mullions");
        assert!(segs.contains(&"h1.2".to_string()) && segs.contains(&"v3.1".to_string()));
        let classes: Vec<String> = g.segments.iter().map(|s| s.cls.clone()).collect();
        assert_eq!(
            classes.iter().filter(|c| *c == "boundary").count(),
            3 + 3 + 2 + 2
        );
        assert_eq!(classes.iter().filter(|c| *c == "mullion").count(), 4);
        assert_eq!(classes.iter().filter(|c| *c == "transom").count(), 3);
        // The front is the right of the baseline: -y for a baseline along +x.
        assert_eq!(
            field(&v, "cells").map(|_| g.runs[0].normal),
            Some([0.0, -1.0, 0.0])
        );
    }

    #[test]
    fn corner_lines_and_restart_per_run() {
        let g = grid(
            "{baseline: [[0, 0], [2.5, 0], [2.5, 1.5]], base: 0, top: 1}",
            r#"{pattern: [{size: 1}]}"#,
            r#"{pattern: [{size: 1}], distribution: "count"}"#,
        );
        let cls: Vec<&str> = g.ulines.iter().map(|l| l.cls.as_str()).collect();
        assert_eq!(
            cls,
            [
                "border", "division", "division", "corner", "division", "border"
            ]
        );
        assert_eq!(g.columns.len(), 3 + 2);
        assert_eq!(g.segments.iter().filter(|s| s.cls == "corner").count(), 1);
    }

    #[test]
    fn spandrel_rows_make_spandrel_cells() {
        let g = grid(
            WALL,
            U3,
            r#"{pattern: [{size: 0.5, cls: "spandrel"}, {size: 1.5, flexible: true}], distribution: "best_division"}"#,
        );
        let cls: Vec<&str> = g.cells.iter().map(|c| c.cls.as_str()).collect();
        assert_eq!(
            cls,
            ["spandrel", "spandrel", "spandrel", "main", "main", "main"]
        );
    }

    #[test]
    fn join_trims_and_panel_insets() {
        let mut g = Grid::build(
            &val(WALL),
            &val(U3),
            &val(r#"{pattern: [{size: 1}], distribution: "count"}"#),
        )
        .unwrap();
        let widths: BTreeMap<String, f64> = [
            ("mullion".into(), 0.06),
            ("boundary".into(), 0.08),
            ("transom".into(), 0.05),
        ]
        .into();
        g.finish("u_continuous", &widths).unwrap();
        let h = g.segments.iter().find(|s| s.id == "h0.1").unwrap();
        assert!((h.trim_start - 0.03).abs() < 1e-12 && (h.trim_end - 0.03).abs() < 1e-12);
        let first = g.segments.iter().find(|s| s.id == "h0.0").unwrap();
        assert!(
            (first.trim_start - 0.04).abs() < 1e-12,
            "boundary mullion at the start"
        );
        let v = g.segments.iter().find(|s| s.id == "v1.0").unwrap();
        assert_eq!(
            (v.trim_start, v.trim_end),
            (0.0, 0.0),
            "mullions run through"
        );
        let c = g.cells.iter().find(|c| c.id == "c1.0").unwrap();
        assert_eq!(c.panel.unwrap().1, [0.04, 0.03, 0.04, 0.03]);
        g.finish("v_continuous", &widths).unwrap();
        let v = g.segments.iter().find(|s| s.id == "v1.0").unwrap();
        assert!((v.trim_start - 0.04).abs() < 1e-12);
    }

    #[test]
    fn overrides_merge_cells_and_keep_orphans() {
        let mut g = Grid::build(
            &val(WALL),
            &val(U3),
            &val(r#"{pattern: [{size: 2}], distribution: "count"}"#),
        )
        .unwrap();
        g.apply_overrides(&val(
            r#"[{remove: "h1.2"}, {cell: "c0.0", cls: "door", component: "opo:library/door"}, {remove: "h0.0"}, {segment: "v9.9", cls: "x"}]"#,
        ));
        g.finish("u_continuous", &BTreeMap::new()).unwrap();
        let v = g.value();
        assert_eq!(ids(&v, "cells"), ["c0.0", "c1.0", "c2.0", "c0.1", "c1.1"]);
        let merged = g.cells.iter().find(|c| c.id == "c2.0").unwrap();
        assert_eq!(merged.rows, 2);
        assert_eq!(g.cells[0].cls, "door");
        match field(&v, "unused_overrides") {
            Some(Value::List(l)) => assert_eq!(l.len(), 2, "a border segment and a missing id"),
            _ => panic!(),
        }
        let side = g.segments.iter().find(|s| s.id == "v3.1").unwrap();
        assert_eq!(side.cells, ["c2.0"]);
    }

    #[test]
    fn sloped_face() {
        let g = grid(
            "{face: [[0, 0, 0], [2, 0, 0], [2, 1, 1], [0, 1, 1]]}",
            r#"{pattern: [{size: 2}], distribution: "count"}"#,
            r#"{pattern: [{size: 1}], distribution: "count"}"#,
        );
        assert_eq!(g.cells.len(), 2);
        let n = g.runs[0].normal;
        let h = 2f64.sqrt() / 2.0;
        assert!((n[0]).abs() < 1e-12 && (n[1] + h).abs() < 1e-12 && (n[2] - h).abs() < 1e-12);
        assert!(
            Grid::build(
                &val("{face: [[0, 0, 0], [2, 0, 0], [3, 1, 1], [0, 1, 1]]}"),
                &val(U3),
                &val(U3)
            )
            .is_err()
        );
    }
}
