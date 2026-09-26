use super::queries::{call_sites, CallSite};
use super::ExprUnit;
use crate::check::name_problem;
use crate::common::error::{arguments, arity_message, CompileError, Span};
use crate::common::limits::{MAX_CALL_DEPTH, MAX_PARAMS};
use crate::eval::{FunctionDef, FunctionTable};
use smol_str::SmolStr;
use std::collections::{HashMap, HashSet};

/// Host functions written in temple, callable from units evaluated with an
/// [`Env`](super::Env) that carries them.
#[derive(Debug, Clone, Default)]
pub struct Functions {
    table: FunctionTable,
}

struct Defined {
    name: SmolStr,
    span: Span,
}

impl Functions {
    /// No functions.
    pub fn new() -> Self {
        Self::default()
    }

    /// Build and check functions from `(name, parameters, body)`. Refuses
    /// reserved or built-in names, duplicates, unknown calls, wrong arity,
    /// recursion, and call chains deeper than 64.
    pub fn build<I, N, P>(definitions: I) -> Result<Functions, Vec<CompileError>>
    where
        I: IntoIterator<Item = (N, Vec<P>, ExprUnit)>,
        N: Into<SmolStr>,
        P: Into<SmolStr>,
    {
        let mut errors = Vec::new();
        let (table, defined) = check_signatures(definitions, &mut errors);
        let callees = resolve_calls(&table, &defined, &mut errors);
        check_call_graph(&callees, &defined, &mut errors);
        if errors.is_empty() {
            Ok(Functions { table })
        } else {
            Err(errors)
        }
    }

    /// How many functions there are.
    pub fn len(&self) -> usize {
        self.table.len()
    }

    /// Whether there are no functions.
    pub fn is_empty(&self) -> bool {
        self.table.is_empty()
    }

    /// Whether a function called `name` is defined.
    pub fn contains(&self, name: &str) -> bool {
        self.table.contains_key(name)
    }

    /// Each function's name and arity, e.g. to build a [`Names`](super::Names).
    pub fn signatures(&self) -> impl Iterator<Item = (&str, usize)> {
        self.table
            .iter()
            .map(|(name, function)| (name.as_str(), function.params.len()))
    }

    pub(crate) fn table(&self) -> &FunctionTable {
        &self.table
    }
}

fn check_signatures<I, N, P>(
    definitions: I,
    errors: &mut Vec<CompileError>,
) -> (FunctionTable, Vec<Defined>)
where
    I: IntoIterator<Item = (N, Vec<P>, ExprUnit)>,
    N: Into<SmolStr>,
    P: Into<SmolStr>,
{
    let mut table = FunctionTable::default();
    let mut defined: Vec<Defined> = Vec::new();
    for (name, params, body) in definitions {
        let name: SmolStr = name.into();
        let span = body.span();
        let params: Vec<SmolStr> = params.into_iter().map(Into::into).collect();
        if let Some(problem) = name_problem(&name, true) {
            errors.push(syntax(problem, span));
        }
        if params.len() > MAX_PARAMS {
            errors.push(syntax(
                format!(
                    "`{name}` takes {} parameters; the most a function may take is {MAX_PARAMS}",
                    params.len()
                ),
                span,
            ));
        }
        let mut seen = HashSet::new();
        for param in &params {
            if let Some(problem) = name_problem(param, false) {
                errors.push(syntax(format!("parameter {problem}"), span));
            } else if !seen.insert(param.clone()) {
                errors.push(syntax(
                    format!("parameter `{param}` appears twice in `{name}`"),
                    span,
                ));
            }
        }
        if table.contains_key(&name) {
            errors.push(syntax(format!("function `{name}` is defined twice"), span));
            continue;
        }
        defined.push(Defined {
            name: name.clone(),
            span,
        });
        table.insert(
            name,
            FunctionDef {
                params,
                body: body.expr,
            },
        );
    }
    (table, defined)
}

/// Returns, for each function, the functions it calls.
fn resolve_calls(
    table: &FunctionTable,
    defined: &[Defined],
    errors: &mut Vec<CompileError>,
) -> Vec<Vec<usize>> {
    let position_by_name: HashMap<&str, usize> = defined
        .iter()
        .enumerate()
        .map(|(position, function)| (function.name.as_str(), position))
        .collect();
    let mut callees: Vec<Vec<usize>> = vec![Vec::new(); defined.len()];
    for (position, function) in defined.iter().enumerate() {
        let mut calls = Vec::new();
        call_sites(&table[&function.name].body, &mut calls);
        for CallSite { name, arity, span } in calls {
            match table.get(&name) {
                Some(callee) if callee.params.len() == arity => {
                    callees[position].push(position_by_name[name.as_str()]);
                }
                Some(callee) => errors.push(syntax(
                    arity_message(&name, &arguments(callee.params.len()), arity),
                    span,
                )),
                None => errors.push(syntax(format!("unknown function `{name}`"), span)),
            }
        }
    }
    callees
}

fn check_call_graph(callees: &[Vec<usize>], defined: &[Defined], errors: &mut Vec<CompileError>) {
    let graph = CallGraph::walk(callees);
    if let Some(cycle) = &graph.cycle {
        let names: Vec<&str> = cycle
            .iter()
            .map(|&position| defined[position].name.as_str())
            .collect();
        errors.push(syntax(
            format!(
                "function `{}` calls itself: {}",
                names[0],
                names.join(" -> ")
            ),
            defined[cycle[0]].span,
        ));
    }
    // one long chain makes every function on it too deep; name a few
    let too_deep =
        (0..defined.len()).filter(|&position| graph.longest_chain_below[position] > MAX_CALL_DEPTH);
    for position in too_deep.take(10) {
        let function = &defined[position];
        errors.push(syntax(
            format!(
                "calls starting at `{}` nest deeper than {MAX_CALL_DEPTH}",
                function.name
            ),
            function.span,
        ));
    }
}

fn syntax(message: String, span: Span) -> CompileError {
    CompileError::Syntax { message, span }
}

struct CallGraph {
    /// Longest call chain below each function (0 if unreached).
    longest_chain_below: Vec<usize>,
    /// First cycle found, ending with the function it returns to.
    cycle: Option<Vec<usize>>,
}

#[derive(Clone, Copy, PartialEq)]
enum Visit {
    NotYet,
    OnPath,
    Done,
}

impl CallGraph {
    /// Walk `callees` with an explicit stack, so long chains can't overflow.
    fn walk(callees: &[Vec<usize>]) -> CallGraph {
        let mut visit = vec![Visit::NotYet; callees.len()];
        let mut longest_chain_below = vec![0usize; callees.len()];
        for start in 0..callees.len() {
            if visit[start] != Visit::NotYet {
                continue;
            }
            // each frame: a function, and how many of its callees are followed
            let mut path: Vec<(usize, usize)> = vec![(start, 0)];
            visit[start] = Visit::OnPath;
            while let Some(top) = path.last_mut() {
                let (function, followed) = *top;
                let Some(&callee) = callees[function].get(followed) else {
                    path.pop();
                    visit[function] = Visit::Done;
                    if let Some(&(caller, _)) = path.last() {
                        longest_chain_below[caller] =
                            longest_chain_below[caller].max(longest_chain_below[function] + 1);
                    }
                    continue;
                };
                top.1 += 1;
                match visit[callee] {
                    Visit::Done => {
                        longest_chain_below[function] =
                            longest_chain_below[function].max(longest_chain_below[callee] + 1);
                    }
                    Visit::OnPath => {
                        let from = path
                            .iter()
                            .position(|&(on_path, _)| on_path == callee)
                            .unwrap_or(0);
                        let mut cycle: Vec<usize> =
                            path[from..].iter().map(|&(on_path, _)| on_path).collect();
                        cycle.push(callee);
                        return CallGraph {
                            longest_chain_below,
                            cycle: Some(cycle),
                        };
                    }
                    Visit::NotYet => {
                        visit[callee] = Visit::OnPath;
                        path.push((callee, 0));
                    }
                }
            }
        }
        CallGraph {
            longest_chain_below,
            cycle: None,
        }
    }
}
