use egg::{rewrite as rw, *};
use fxhash::FxHashSet as HashSet;
use fxhash::FxHashMap as HashMap;

define_language! {
    enum Lambda {
        Bool(bool),
        Num(i32),

        "var" = Var(Id),

        "+" = Add([Id; 2]),
        "=" = Eq([Id; 2]),

        "app" = App([Id; 2]),
        "lam" = Lambda([Id; 2]),
        "let" = Let([Id; 3]),
        "fix" = Fix([Id; 2]),

        "if" = If([Id; 3]),

        Symbol(egg::Symbol),
    }
}

impl Lambda {
    fn num(&self) -> Option<i32> {
        match self {
            Lambda::Num(n) => Some(*n),
            _ => None,
        }
    }
}

type EGraph = egg::EGraph<Lambda, LambdaAnalysis>;

#[derive(Default)]
struct LambdaAnalysis;

#[derive(Debug)]
struct Data {
    free: HashSet<Id>,
    constant: Option<(Lambda, PatternAst<Lambda>)>,
}

fn eval(egraph: &EGraph, enode: &Lambda) -> Option<(Lambda, PatternAst<Lambda>)> {
    let x = |i: &Id| egraph[*i].data.constant.as_ref().map(|c| &c.0);
    match enode {
        Lambda::Num(n) => Some((enode.clone(), format!("{}", n).parse().unwrap())),
        Lambda::Bool(b) => Some((enode.clone(), format!("{}", b).parse().unwrap())),
        Lambda::Add([a, b]) => Some((
            Lambda::Num(x(a)?.num()? + x(b)?.num()?),
            format!("(+ {} {})", x(a)?, x(b)?).parse().unwrap(),
        )),
        Lambda::Eq([a, b]) => Some((
            Lambda::Bool(x(a)? == x(b)?),
            format!("(= {} {})", x(a)?, x(b)?).parse().unwrap(),
        )),
        _ => None,
    }
}

impl Analysis<Lambda> for LambdaAnalysis {
    type Data = Data;
    fn merge(&mut self, to: &mut Data, from: Data) -> DidMerge {
        let before_len = to.free.len();
        // to.free.extend(from.free);
        to.free.retain(|i| from.free.contains(i));
        // compare lengths to see if I changed to or from
        DidMerge(
            before_len != to.free.len(),
            to.free.len() != from.free.len(),
        ) | merge_option(&mut to.constant, from.constant, |a, b| {
            assert_eq!(a.0, b.0, "Merged non-equal constants");
            DidMerge(false, false)
        })
    }

    fn make(egraph: &EGraph, enode: &Lambda) -> Data {
        let f = |i: &Id| egraph[*i].data.free.iter().cloned();
        let mut free = HashSet::default();
        match enode {
            Lambda::Var(v) => {
                free.insert(*v);
            }
            Lambda::Let([v, a, b]) => {
                free.extend(f(b));
                free.remove(v);
                free.extend(f(a));
            }
            Lambda::Lambda([v, a]) | Lambda::Fix([v, a]) => {
                free.extend(f(a));
                free.remove(v);
            }
            _ => enode.for_each(|c| free.extend(&egraph[c].data.free)),
        }
        let constant = eval(egraph, enode);
        Data { constant, free }
    }

    fn modify(egraph: &mut EGraph, id: Id) {
        if let Some(c) = egraph[id].data.constant.clone() {
            if egraph.are_explanations_enabled() {
                egraph.union_instantiations(
                    &c.0.to_string().parse().unwrap(),
                    &c.1,
                    &Default::default(),
                    "analysis".to_string(),
                );
            } else {
                let const_id = egraph.add(c.0);
                egraph.union(id, const_id);
            }
        }
    }
}

fn var(s: &str) -> Var {
    s.parse().unwrap()
}

fn is_not_same_var(v1: Var, v2: Var) -> impl Fn(&mut EGraph, Id, &Subst) -> bool {
    move |egraph, _, subst| egraph.find(subst[v1]) != egraph.find(subst[v2])
}

fn is_const(v: Var) -> impl Fn(&mut EGraph, Id, &Subst) -> bool {
    move |egraph, _, subst| egraph[subst[v]].data.constant.is_some()
}

fn rules() -> Vec<Rewrite<Lambda, LambdaAnalysis>> {
    vec![
        // open term rules
        rw!("if-true";  "(if  true ?then ?else)" => "?then"),
        rw!("if-false"; "(if false ?then ?else)" => "?else"),
        rw!("if-elim"; "(if (= (var ?x) ?e) ?then ?else)" => "?else"
            if ConditionEqual::parse("(let ?x ?e ?then)", "(let ?x ?e ?else)")),
        rw!("add-comm";  "(+ ?a ?b)"        => "(+ ?b ?a)"),
        rw!("add-assoc"; "(+ (+ ?a ?b) ?c)" => "(+ ?a (+ ?b ?c))"),
        rw!("eq-comm";   "(= ?a ?b)"        => "(= ?b ?a)"),
        // subst rules
        rw!("fix";      "(fix ?v ?e)"             => "(let ?v (fix ?v ?e) ?e)"),
        // rw!("beta";     "(app (lam ?v ?body) ?e)" => "(let ?v ?e ?body)"),
        rw!("beta";     "(app (lam ?v ?body) ?e)" => {
            { CallByName {
                v: var("?v"),
                e: var("?e"),
                body: var("?body"),
            }}
        }),
        rw!("let-app";  "(let ?v ?e (app ?a ?b))" => "(app (let ?v ?e ?a) (let ?v ?e ?b))"),
        rw!("let-add";  "(let ?v ?e (+   ?a ?b))" => "(+   (let ?v ?e ?a) (let ?v ?e ?b))"),
        rw!("let-eq";   "(let ?v ?e (=   ?a ?b))" => "(=   (let ?v ?e ?a) (let ?v ?e ?b))"),
        rw!("let-const";
            "(let ?v ?e ?c)" => "?c" if is_const(var("?c"))),
        rw!("let-if";
            "(let ?v ?e (if ?cond ?then ?else))" =>
            "(if (let ?v ?e ?cond) (let ?v ?e ?then) (let ?v ?e ?else))"
        ),
        rw!("let-var-same"; "(let ?v1 ?e (var ?v1))" => "?e"),
        rw!("let-var-diff"; "(let ?v1 ?e (var ?v2))" => "(var ?v2)"
            if is_not_same_var(var("?v1"), var("?v2"))),
        rw!("let-lam-same"; "(let ?v1 ?e (lam ?v1 ?body))" => "(lam ?v1 ?body)"),
        rw!("let-lam-diff";
            "(let ?v1 ?e (lam ?v2 ?body))" =>
            { CaptureAvoid {
                fresh: var("?fresh"), v2: var("?v2"), e: var("?e"),
                if_not_free: "(lam ?v2 (let ?v1 ?e ?body))".parse().unwrap(),
                if_free: "(lam ?fresh (let ?v1 ?e (let ?v2 (var ?fresh) ?body)))".parse().unwrap(),
            }}
            if is_not_same_var(var("?v1"), var("?v2"))),
    ]
}

// fn substitute(egraph: &mut EGraph, expr: RecExpr<Lambda>, sym: Symbol, subst_expr: RecExpr<Lambda>) -> (Id, Vec<Id>) {
//     panic!("expr: {:?}, sym: {}, subst_expr: {:?}", expr.as_ref(), sym, subst_expr.as_ref())
// }

struct SketchGuidedBetaReduction {
    v: Var,
    e: Var,
    body: Var
}

impl Applier<Lambda, LambdaAnalysis> for SketchGuidedBetaReduction {
    fn apply_one(
        &self,
        egraph: &mut EGraph,
        eclass: Id,
        subst: &Subst,
        _searcher_ast: Option<&PatternAst<Lambda>>,
        _rule_name: Symbol,
    ) -> Vec<Id> {
        let v = subst[self.v];
        let sym_to_replace = get_sym(v, egraph);
        let e = subst[self.e];
        let body = subst[self.body];
        let extractor = Extractor::new(&egraph, AstSize);
        let (_, best_e) = extractor.find_best(e);
        let (_, best_body) = extractor.find_best(body);
        panic!()
        // let (new_id, changed_ids) = substitute(egraph, best_body, sym_to_replace, best_e);
        // egraph.union(eclass, new_id);
        // vec!(new_id) // + changed_ids
    }
}

struct CallByName {
    v: Var,
    e: Var,
    body: Var,
}
fn substitute(
    egraph: &mut EGraph,
    subst_sym: Symbol,
    subst_e: Id,
    target_eclass: Id,
    memo: &mut HashMap<Id, Option<Vec<Id>>>,
) -> Vec<Id> {
    // if !egraph[target_eclass].data.free.contains(&v) {

    //     return vec!()
    // }
    if let Some(result) = memo.get(&target_eclass) {
        match result {
            Some(cached_value) => return cached_value.to_vec(),
            None => {
                // infinite loop
                // panic!("infinite loop at id: {:?}, egraph: {:?}", subst_e, egraph);
                return vec!()
            }
        }
    }
    // println!("eclass_id: {:?}, starting search", target_eclass);
    memo.insert(target_eclass, None);
    let mut new_ids = vec!();
    for lambda_term in egraph[target_eclass].nodes.clone() {
        let mut ids = match lambda_term {
            Lambda::App([e1, e2]) => {
                let subst_e1s = substitute(egraph, subst_sym, subst_e, e1, memo);
                let subst_e2s = substitute(egraph, subst_sym, subst_e, e2, memo);
                product(&subst_e1s, &subst_e2s)
                    .map(|(e1, e2)| egraph.add(Lambda::App([*e1, *e2])))
                    .collect()
            }
            Lambda::Add([e1, e2]) => {
                let subst_e1s = substitute(egraph, subst_sym, subst_e, e1, memo);
                let subst_e2s = substitute(egraph, subst_sym, subst_e, e2, memo);
                product(&subst_e1s, &subst_e2s)
                    .map(|(e1, e2)| egraph.add(Lambda::Add([*e1, *e2])))
                    .collect()
            }
            Lambda::Lambda([e1, e2]) => {
                let sym = get_sym(e1, egraph);
                // Can't substitute
                if sym == subst_sym {
                    vec!()
                } else if false && egraph[subst_e].data.free.contains(&e1) {
                    // This way of getting a fresh sym is stolen from
                    // CaptureAvoid, hopefully it is OK.
                    let fresh_sym = Lambda::Symbol(format!("_{}", target_eclass).into());
                    let fresh_sym_id = egraph.add(fresh_sym);
                    let fresh_sym_var_id = egraph.add(Lambda::Var(fresh_sym_id));
                    let fresh_e2s = substitute(egraph, sym, fresh_sym_var_id, e2, &mut HashMap::default());
                    // Can't be done without a termporary variable because
                    // egraph would be borrowed twice. Probably better to not
                    // collect and use an iterator.
                    let subst_e2s: Vec<Id> = fresh_e2s
                        .iter()
                        .flat_map(|fresh_e2| substitute(egraph, subst_sym, subst_e, *fresh_e2, memo))
                        .collect();
                    subst_e2s
                        .iter()
                        .map(|subst_e2| egraph.add(Lambda::Lambda([fresh_sym_var_id, *subst_e2])))
                        .collect()
                    // Should this be done?
                    // let fresh_lam = egraph.add(Lambda::Lambda([fresh_sym_var_id, fresh_e2]));
                    // Alpha equivalent
                    // egraph.union(target_eclass, fresh_lam);

                } else {
                    substitute(egraph, subst_sym, subst_e, e2, memo)
                        .iter()
                        .map(|subst_e2| egraph.add(Lambda::Lambda([e1, *subst_e2])))
                        .collect()
                }
            }
            Lambda::Let([e1, e2, e3]) => {
                let sym = get_sym(e1, egraph);
                // Can't substitute
                if sym == subst_sym {
                    vec!()
                } else {
                let subst_e2s = substitute(egraph, subst_sym, subst_e, e2, memo);
                let subst_e3s = substitute(egraph, subst_sym, subst_e, e3, memo);
                product(&subst_e2s, &subst_e3s)
                    .map(|(e2, e3)| egraph.add(Lambda::Let([e1, *e2, *e3])))
                    .collect()
                }
            }
            Lambda::Var(id) => {
                match egraph[id].nodes[..] {
                    [Lambda::Symbol(sym)] => {
                        if sym == subst_sym {
                            vec!(subst_e)
                        } else {
                            vec!(target_eclass)
                        }
                    },
                    _ => {
                        substitute(egraph, subst_sym, subst_e, id, memo)
                            .iter()
                            .map(|e| egraph.add(Lambda::Var(*e)))
                            .collect()
                    }
                }
            }
            _ => vec!(target_eclass)
        };
        new_ids.append(&mut ids);
    }
    for (class1, class2) in product(&new_ids, &new_ids) {
        egraph.union(*class1, *class2);
    }
    // println!("eclass_id: {:?}, new_ids: {:?}", target_eclass, new_ids.clone());
    memo.insert(target_eclass, Some(new_ids.clone()));
    new_ids
}

impl Applier<Lambda, LambdaAnalysis> for CallByName {
    fn apply_one(
        &self,
        egraph: &mut EGraph,
        eclass: Id,
        subst: &Subst,
        _searcher_ast: Option<&PatternAst<Lambda>>,
        _rule_name: Symbol,
    ) -> Vec<Id> {
        let subst_sym = get_sym(subst[self.v], egraph);
        let new_ids = substitute(egraph, subst_sym, subst[self.e], subst[self.body], &mut HashMap::default());
        for id in &new_ids {
            egraph.union(eclass, *id);
        }
        // println!("eclass: {:?}, subst_sym: {:?}, subst_e: {:?}, subst_body: {:?}, new_ids: {:?}\negraph: {:?}", eclass, subst_sym, subst[self.e], subst[self.body], new_ids.clone(), egraph);
        new_ids
    }
}

fn get_sym(eclass: Id, egraph: &EGraph) -> Symbol {
    let nodes = &egraph[eclass].nodes;
    // This var should just point to a symbol
    match nodes[..] {
        [Lambda::Symbol(sym)] => sym,
        _ => panic!("Nodes at id: {:?} are not just a single symbol, nodes: {:?}", eclass, nodes)
    }
}


// https://stackoverflow.com/questions/69613407/how-do-i-get-the-cartesian-product-of-2-vectors-by-using-iterator/74805365#74805365
fn product<'a: 'c, 'b: 'c, 'c, T>(
    xs: &'a [T],
    ys: &'b [T],
) -> impl Iterator<Item = (&'a T, &'b T)> + 'c {
    xs.iter().flat_map(move |x| std::iter::repeat(x).zip(ys))
}

struct CaptureAvoid {
    fresh: Var,
    v2: Var,
    e: Var,
    if_not_free: Pattern<Lambda>,
    if_free: Pattern<Lambda>,
}

impl Applier<Lambda, LambdaAnalysis> for CaptureAvoid {
    fn apply_one(
        &self,
        egraph: &mut EGraph,
        eclass: Id,
        subst: &Subst,
        searcher_ast: Option<&PatternAst<Lambda>>,
        rule_name: Symbol,
    ) -> Vec<Id> {
        let e = subst[self.e];
        let v2 = subst[self.v2];
        let v2_free_in_e = egraph[e].data.free.contains(&v2);
        if v2_free_in_e {
            let mut subst = subst.clone();
            let sym = Lambda::Symbol(format!("_{}", eclass).into());
            subst.insert(self.fresh, egraph.add(sym));
            self.if_free
                .apply_one(egraph, eclass, &subst, searcher_ast, rule_name)
        } else {
            self.if_not_free
                .apply_one(egraph, eclass, subst, searcher_ast, rule_name)
        }
    }
}

egg::test_fn! {
    lambda_under, rules(),
    "(lam x (+ 4
               (app (lam y (var y))
                    4)))"
    =>
    // "(lam x (+ 4 (let y 4 (var y))))",
    // "(lam x (+ 4 4))",
    "(lam x 8))",
}

egg::test_fn! {
    lambda_if_elim, rules(),
    "(if (= (var a) (var b))
         (+ (var a) (var a))
         (+ (var a) (var b)))"
    =>
    "(+ (var a) (var b))"
}

egg::test_fn! {
    lambda_let_simple, rules(),
    "(let x 0
     (let y 1
     (+ (var x) (var y))))"
    =>
    // "(let ?a 0
    //  (+ (var ?a) 1))",
    // "(+ 0 1)",
    "1",
}

egg::test_fn! {
    #[should_panic(expected = "Could not prove goal 0")]
    lambda_capture, rules(),
    "(let x 1 (lam x (var x)))" => "(lam x 1)"
}

egg::test_fn! {
    #[should_panic(expected = "Could not prove goal 0")]
    lambda_capture_free, rules(),
    "(let y (+ (var x) (var x)) (lam x (var y)))" => "(lam x (+ (var x) (var x)))"
}

egg::test_fn! {
    #[should_panic(expected = "Could not prove goal 0")]
    lambda_closure_not_seven, rules(),
    "(let five 5
     (let add-five (lam x (+ (var x) (var five)))
     (let five 6
     (app (var add-five) 1))))"
    =>
    "7"
}

egg::test_fn! {
    lambda_compose, rules(),
    "(let compose (lam f (lam g (lam x (app (var f)
                                       (app (var g) (var x))))))
     (let add1 (lam y (+ (var y) 1))
     (app (app (var compose) (var add1)) (var add1))))"
    =>
    "(lam ?x (+ 1
                (app (lam ?y (+ 1 (var ?y)))
                     (var ?x))))",
    "(lam ?x (+ (var ?x) 2))"
}

egg::test_fn! {
    lambda_if_simple, rules(),
    "(if (= 1 1) 7 9)" => "7"
}

// Times out
// (without a double, takes ~20s)
egg::test_fn! {
    lambda_compose_many_many1, rules(),
    "(let compose (lam f (lam g (lam x (app (var f)
                                       (app (var g) (var x))))))
     (let double (lam f (app (app (var compose) (var f)) (var f)))
     (let add1 (lam y (+ (var y) 1))
     (app (var double) 
     (app (var double)
     (app (var double)
     (app (var double)
     (app (var double)
     (app (var double)
     (app (var double)
     (app (var double)
         (var add1))))))))))))"
    =>
    "(lam ?x (+ (var ?x) 256))"
}

// Times out
// (without a double, takes ~20s)
egg::test_fn! {
    lambda_compose_many_many2, rules(),
    "(let compose (lam f (lam g (lam x (app (var f)
                                       (app (var g) (var x))))))
     (let double (lam f (app (app (var compose) (var f)) (var f)))
     (let add1 (lam y (+ (var y) 1))
     (app
         (app (var double) 
         (app (var double)
         (app (var double)
              (var double))))
         (var add1)))))"
    =>
    "(lam ?x (+ (var ?x) 32))"
}

egg::test_fn! {
    lambda_call_by_name_1, rules(),
    "(let double (lam f (lam x (app (var f) (app (var f) (var x)))))
     (let add1 (lam y (+ (var y) 1))
     (app (var double)
         (var add1))))"
    =>
    "(lam ?x (+ (var ?x) 2))"
}

egg::test_fn! {
    lambda_call_by_name_2, rules(),
    "(let compose (lam f (lam g (lam x (app (var f)
                                       (app (var g) (var x))))))
     (let double (lam f (app (app (var compose) (var f)) (var f)))
     (let add1 (lam y (+ (var y) 1))
     (app (var double)
         (var add1)))))"
    =>
    "(lam ?x (+ (var ?x) 2))"
}

egg::test_fn! {
    lambda_call_by_name_3, rules(),
    "(let compose (lam f (lam g (lam x (app (var f)
                                       (app (var g) (var x))))))
     (let double (lam f (app (app (var compose) (var f)) (var f)))
     (let add1 (lam y (+ (var y) 1))
     (app
         (app (var double)
              (var double))
         (var add1)))))"
    =>
    "(lam ?x (+ (var ?x) 4))"
}

egg::test_fn! {
    lambda_call_by_name_4, rules(),
    "(let compose (lam f (lam g (lam x (app (var f)
                                       (app (var g) (var x))))))
     (let add1 (lam y (+ (var y) 1))
     (let addadd1 (lam f (app (app (var compose) (var add1)) (var f)))
     (app (var addadd1)
     (app (var addadd1)
         (var add1))))))"
    =>
    "(lam ?x (+ (var ?x) 3))"
}

egg::test_fn! {
    #[cfg(not(debug_assertions))]
    #[cfg_attr(feature = "test-explanations", ignore)]
    lambda_function_repeat, rules(),
    runner = Runner::default()
        .with_time_limit(std::time::Duration::from_secs(20))
        .with_node_limit(150_000)
        .with_iter_limit(60),
    "(let compose (lam f (lam g (lam x (app (var f)
                                       (app (var g) (var x))))))
     (let repeat (fix repeat (lam fun (lam n
        (if (= (var n) 0)
            (lam i (var i))
            (app (app (var compose) (var fun))
                 (app (app (var repeat)
                           (var fun))
                      (+ (var n) -1)))))))
     (let add1 (lam y (+ (var y) 1))
     (app (app (var repeat)
               (var add1))
          2))))"
    =>
    "(lam ?x (+ (var ?x) 2))"
}

egg::test_fn! {
    lambda_if, rules(),
    "(let zeroone (lam x
        (if (= (var x) 0)
            0
            1))
        (+ (app (var zeroone) 0)
        (app (var zeroone) 10)))"
    =>
    // "(+ (if false 0 1) (if true 0 1))",
    // "(+ 1 0)",
    "1",
}

egg::test_fn! {
    #[cfg(not(debug_assertions))]
    #[cfg_attr(feature = "test-explanations", ignore)]
    lambda_fib, rules(),
    runner = Runner::default()
        .with_iter_limit(60)
        .with_node_limit(500_000),
    "(let fib (fix fib (lam n
        (if (= (var n) 0)
            0
        (if (= (var n) 1)
            1
        (+ (app (var fib)
                (+ (var n) -1))
            (app (var fib)
                (+ (var n) -2)))))))
        (app (var fib) 4))"
    => "3"
}

#[test]
fn lambda_ematching_bench() {
    let exprs = &[
        "(let zeroone (lam x
            (if (= (var x) 0)
                0
                1))
            (+ (app (var zeroone) 0)
            (app (var zeroone) 10)))",
        "(let compose (lam f (lam g (lam x (app (var f)
                                        (app (var g) (var x))))))
        (let repeat (fix repeat (lam fun (lam n
            (if (= (var n) 0)
                (lam i (var i))
                (app (app (var compose) (var fun))
                    (app (app (var repeat)
                            (var fun))
                        (+ (var n) -1)))))))
        (let add1 (lam y (+ (var y) 1))
        (app (app (var repeat)
                (var add1))
            2))))",
        "(let fib (fix fib (lam n
            (if (= (var n) 0)
                0
            (if (= (var n) 1)
                1
            (+ (app (var fib)
                    (+ (var n) -1))
                (app (var fib)
                    (+ (var n) -2)))))))
            (app (var fib) 4))",
    ];

    let extra_patterns = &[
        "(if (= (var ?x) ?e) ?then ?else)",
        "(+ (+ ?a ?b) ?c)",
        "(let ?v (fix ?v ?e) ?e)",
        "(app (lam ?v ?body) ?e)",
        "(let ?v ?e (app ?a ?b))",
        "(app (let ?v ?e ?a) (let ?v ?e ?b))",
        "(let ?v ?e (+   ?a ?b))",
        "(+   (let ?v ?e ?a) (let ?v ?e ?b))",
        "(let ?v ?e (=   ?a ?b))",
        "(=   (let ?v ?e ?a) (let ?v ?e ?b))",
        "(let ?v ?e (if ?cond ?then ?else))",
        "(if (let ?v ?e ?cond) (let ?v ?e ?then) (let ?v ?e ?else))",
        "(let ?v1 ?e (var ?v1))",
        "(let ?v1 ?e (var ?v2))",
        "(let ?v1 ?e (lam ?v1 ?body))",
        "(let ?v1 ?e (lam ?v2 ?body))",
        "(lam ?v2 (let ?v1 ?e ?body))",
        "(lam ?fresh (let ?v1 ?e (let ?v2 (var ?fresh) ?body)))",
    ];

    egg::test::bench_egraph("lambda", rules(), exprs, extra_patterns);
}
