use super::{Type, TypeVar};
use crate::{
    ast::{self, TypedModule},
    docvec,
    pretty::{nil, *},
};
use ecow::EcoString;
use std::sync::Arc;

#[cfg(test)]
use super::*;
#[cfg(test)]
use std::cell::RefCell;

#[cfg(test)]
use pretty_assertions::assert_eq;

const INDENT: isize = 2;

#[derive(Debug)]
pub struct Import {
    module: EcoString,
    renaming: Option<EcoString>,
    unqualified_types: Vec<UnqualifiedImport>,
}

#[derive(Debug)]
pub struct UnqualifiedImport {
    name: EcoString,
    as_name: Option<EcoString>,
}

impl From<&ast::Import<EcoString>> for Import {
    fn from(import_: &ast::Import<EcoString>) -> Self {
        Self {
            module: import_.module.clone(),
            renaming: import_.as_name.clone().and_then(|(n, _)| match n {
                ast::AssignName::Variable(name) => Some(name.into()),
                ast::AssignName::Discard(_) => None,
            }),
            unqualified_types: import_
                .unqualified_types
                .iter()
                .map(|u| UnqualifiedImport {
                    name: u.name.clone(),
                    as_name: u.as_name.clone(),
                })
                .collect(),
        }
    }
}

impl From<&TypedModule> for Vec<Import> {
    fn from(module: &TypedModule) -> Self {
        module
            .definitions
            .iter()
            .filter_map(|d| match d {
                crate::ast::Definition::Import(i) => Some(i),
                _ => None,
            })
            .map(|i| i.into())
            .collect()
    }
}

#[derive(Debug)]
struct ImportContext {
    module: EcoString,
    imports: Vec<Import>,
}

#[derive(Debug, Default)]
pub struct Printer {
    names: im::HashMap<u64, EcoString>,
    uid: u64,
    // A mapping of printd type names to the module that they are defined in.
    printed_types: im::HashMap<EcoString, EcoString>,

    context: Option<ImportContext>,
}

impl Printer {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn with_imports_context(&mut self, module: EcoString, imports: Vec<Import>) {
        self.context = Some(ImportContext { module, imports });
    }

    pub fn with_names(&mut self, names: im::HashMap<u64, EcoString>) {
        self.names = names;
    }

    /// Render a Type as a well formatted string.
    ///
    pub fn pretty_print(&mut self, type_: &Type, initial_indent: usize) -> String {
        let mut buffer = String::with_capacity(initial_indent);
        for _ in 0..initial_indent {
            buffer.push(' ');
        }
        buffer
            .to_doc()
            .append(self.print(type_))
            .nest(initial_indent as isize)
            .to_pretty_string(80)
    }

    // TODO: have this function return a Document that borrows from the Type.
    // Is this possible? The lifetime would have to go through the Arc<Refcell<Type>>
    // for TypeVar::Link'd types.
    pub fn print<'a>(&mut self, type_: &Type) -> Document<'a> {
        match type_ {
            Type::Named {
                name, args, module, ..
            } => {
                let doc = match &self.context {
                    Some(ctx) => {
                        if module == "gleam" || &ctx.module == module {
                            Document::String(name.into())
                        } else {
                            let import_ = ctx.imports.iter().find(|i| &i.module == module);

                            if let Some(import_) = import_ {
                                let renamed_unqualified_import =
                                    import_.unqualified_types.iter().find(|u| &u.name == name);

                                if let Some(u) = renamed_unqualified_import {
                                    return Document::String(match u.as_name {
                                        Some(ref renaming) => renaming.into(),
                                        None => name.into(),
                                    });
                                }
                            }

                            let renaming = import_.and_then(|i| i.renaming.as_deref());

                            let qualifier = renaming.unwrap_or(module);
                            qualify_type_name(qualifier, name)
                        }
                    }
                    None => {
                        if self.name_clashes_if_unqualified(name, module) {
                            qualify_type_name(module, name)
                        } else {
                            let _ = self.printed_types.insert(name.clone(), module.clone());
                            name.to_doc()
                        }
                    }
                };

                if args.is_empty() {
                    doc
                } else {
                    doc.append("(")
                        .append(self.args_to_gleam_doc(args))
                        .append(")")
                }
            }

            Type::Fn { args, retrn } => "fn("
                .to_doc()
                .append(self.args_to_gleam_doc(args))
                .append(") ->")
                .append(
                    break_("", " ")
                        .append(self.print(retrn))
                        .nest(INDENT)
                        .group(),
                ),

            Type::Var { type_, .. } => self.type_var_doc(&type_.borrow()),

            Type::Tuple { elems, .. } => self.args_to_gleam_doc(elems).surround("#(", ")"),
        }
    }

    fn name_clashes_if_unqualified(&mut self, type_: &EcoString, module: &str) -> bool {
        match self.printed_types.get(type_) {
            None => false,
            Some(previous_module) if module == previous_module => false,
            Some(_different_module) => true,
        }
    }

    fn type_var_doc<'a>(&mut self, type_: &TypeVar) -> Document<'a> {
        match type_ {
            TypeVar::Link { ref type_, .. } => self.print(type_),
            TypeVar::Unbound { id, .. } | TypeVar::Generic { id, .. } => self.generic_type_var(*id),
        }
    }

    pub fn generic_type_var<'a>(&mut self, id: u64) -> Document<'a> {
        match self.names.get(&id) {
            Some(n) => {
                let _ = self.printed_types.insert(n.clone(), "".into());
                n.to_doc()
            }
            None => {
                let n = self.next_letter();
                let _ = self.names.insert(id, n.clone());
                let _ = self.printed_types.insert(n.clone(), "".into());
                n.to_doc()
            }
        }
    }

    fn next_letter(&mut self) -> EcoString {
        let alphabet_length = 26;
        let char_offset = 97;
        let mut chars = vec![];
        let mut n;
        let mut rest = self.uid;

        loop {
            n = rest % alphabet_length;
            rest /= alphabet_length;
            chars.push((n as u8 + char_offset) as char);

            if rest == 0 {
                break;
            }
            rest -= 1
        }

        self.uid += 1;
        chars.into_iter().rev().collect()
    }

    fn args_to_gleam_doc(&mut self, args: &[Arc<Type>]) -> Document<'static> {
        if args.is_empty() {
            return nil();
        }

        let args = join(
            args.iter().map(|t| self.print(t).group()),
            break_(",", ", "),
        );
        break_("", "")
            .append(args)
            .nest(INDENT)
            .append(break_(",", ""))
            .group()
    }
}

fn qualify_type_name(module: &str, type_name: &str) -> Document<'static> {
    docvec![EcoString::from(module), ".", EcoString::from(type_name)]
}

#[test]
fn next_letter_test() {
    let mut printer = Printer::new();
    assert_eq!(printer.next_letter().as_str(), "a");
    assert_eq!(printer.next_letter().as_str(), "b");
    assert_eq!(printer.next_letter().as_str(), "c");
    assert_eq!(printer.next_letter().as_str(), "d");
    assert_eq!(printer.next_letter().as_str(), "e");
    assert_eq!(printer.next_letter().as_str(), "f");
    assert_eq!(printer.next_letter().as_str(), "g");
    assert_eq!(printer.next_letter().as_str(), "h");
    assert_eq!(printer.next_letter().as_str(), "i");
    assert_eq!(printer.next_letter().as_str(), "j");
    assert_eq!(printer.next_letter().as_str(), "k");
    assert_eq!(printer.next_letter().as_str(), "l");
    assert_eq!(printer.next_letter().as_str(), "m");
    assert_eq!(printer.next_letter().as_str(), "n");
    assert_eq!(printer.next_letter().as_str(), "o");
    assert_eq!(printer.next_letter().as_str(), "p");
    assert_eq!(printer.next_letter().as_str(), "q");
    assert_eq!(printer.next_letter().as_str(), "r");
    assert_eq!(printer.next_letter().as_str(), "s");
    assert_eq!(printer.next_letter().as_str(), "t");
    assert_eq!(printer.next_letter().as_str(), "u");
    assert_eq!(printer.next_letter().as_str(), "v");
    assert_eq!(printer.next_letter().as_str(), "w");
    assert_eq!(printer.next_letter().as_str(), "x");
    assert_eq!(printer.next_letter().as_str(), "y");
    assert_eq!(printer.next_letter().as_str(), "z");
    assert_eq!(printer.next_letter().as_str(), "aa");
    assert_eq!(printer.next_letter().as_str(), "ab");
    assert_eq!(printer.next_letter().as_str(), "ac");
    assert_eq!(printer.next_letter().as_str(), "ad");
    assert_eq!(printer.next_letter().as_str(), "ae");
    assert_eq!(printer.next_letter().as_str(), "af");
    assert_eq!(printer.next_letter().as_str(), "ag");
    assert_eq!(printer.next_letter().as_str(), "ah");
    assert_eq!(printer.next_letter().as_str(), "ai");
    assert_eq!(printer.next_letter().as_str(), "aj");
    assert_eq!(printer.next_letter().as_str(), "ak");
    assert_eq!(printer.next_letter().as_str(), "al");
    assert_eq!(printer.next_letter().as_str(), "am");
    assert_eq!(printer.next_letter().as_str(), "an");
    assert_eq!(printer.next_letter().as_str(), "ao");
    assert_eq!(printer.next_letter().as_str(), "ap");
    assert_eq!(printer.next_letter().as_str(), "aq");
    assert_eq!(printer.next_letter().as_str(), "ar");
    assert_eq!(printer.next_letter().as_str(), "as");
    assert_eq!(printer.next_letter().as_str(), "at");
    assert_eq!(printer.next_letter().as_str(), "au");
    assert_eq!(printer.next_letter().as_str(), "av");
    assert_eq!(printer.next_letter().as_str(), "aw");
    assert_eq!(printer.next_letter().as_str(), "ax");
    assert_eq!(printer.next_letter().as_str(), "ay");
    assert_eq!(printer.next_letter().as_str(), "az");
    assert_eq!(printer.next_letter().as_str(), "ba");
    assert_eq!(printer.next_letter().as_str(), "bb");
    assert_eq!(printer.next_letter().as_str(), "bc");
    assert_eq!(printer.next_letter().as_str(), "bd");
    assert_eq!(printer.next_letter().as_str(), "be");
    assert_eq!(printer.next_letter().as_str(), "bf");
    assert_eq!(printer.next_letter().as_str(), "bg");
    assert_eq!(printer.next_letter().as_str(), "bh");
    assert_eq!(printer.next_letter().as_str(), "bi");
    assert_eq!(printer.next_letter().as_str(), "bj");
    assert_eq!(printer.next_letter().as_str(), "bk");
    assert_eq!(printer.next_letter().as_str(), "bl");
    assert_eq!(printer.next_letter().as_str(), "bm");
    assert_eq!(printer.next_letter().as_str(), "bn");
    assert_eq!(printer.next_letter().as_str(), "bo");
    assert_eq!(printer.next_letter().as_str(), "bp");
    assert_eq!(printer.next_letter().as_str(), "bq");
    assert_eq!(printer.next_letter().as_str(), "br");
    assert_eq!(printer.next_letter().as_str(), "bs");
    assert_eq!(printer.next_letter().as_str(), "bt");
    assert_eq!(printer.next_letter().as_str(), "bu");
    assert_eq!(printer.next_letter().as_str(), "bv");
    assert_eq!(printer.next_letter().as_str(), "bw");
    assert_eq!(printer.next_letter().as_str(), "bx");
    assert_eq!(printer.next_letter().as_str(), "by");
    assert_eq!(printer.next_letter().as_str(), "bz");
}

#[test]
fn pretty_print_test() {
    macro_rules! assert_string {
        ($src:expr, $type_:expr $(,)?) => {
            let mut printer = Printer::new();
            assert_eq!($type_.to_string(), printer.pretty_print(&$src, 0),);
        };
    }

    assert_string!(
        Type::Named {
            module: "whatever".into(),
            package: "whatever".into(),
            name: "Int".into(),
            publicity: Publicity::Public,
            args: vec![],
        },
        "Int",
    );
    assert_string!(
        Type::Named {
            module: "themodule".into(),
            package: "whatever".into(),
            name: "Pair".into(),
            publicity: Publicity::Public,
            args: vec![
                Arc::new(Type::Named {
                    module: "whatever".into(),
                    package: "whatever".into(),
                    name: "Int".into(),
                    publicity: Publicity::Public,
                    args: vec![],
                }),
                Arc::new(Type::Named {
                    module: "whatever".into(),
                    package: "whatever".into(),
                    name: "Bool".into(),
                    publicity: Publicity::Public,
                    args: vec![],
                }),
            ],
        },
        "Pair(Int, Bool)",
    );
    assert_string!(
        Type::Fn {
            args: vec![
                Arc::new(Type::Named {
                    args: vec![],
                    module: "whatever".into(),
                    package: "whatever".into(),
                    name: "Int".into(),
                    publicity: Publicity::Public,
                }),
                Arc::new(Type::Named {
                    args: vec![],
                    module: "whatever".into(),
                    package: "whatever".into(),
                    name: "Bool".into(),
                    publicity: Publicity::Public,
                }),
            ],
            retrn: Arc::new(Type::Named {
                args: vec![],
                module: "whatever".into(),
                package: "whatever".into(),
                name: "Bool".into(),
                publicity: Publicity::Public,
            }),
        },
        "fn(Int, Bool) -> Bool",
    );
    assert_string!(
        Type::Var {
            type_: Arc::new(RefCell::new(TypeVar::Link {
                type_: Arc::new(Type::Named {
                    args: vec![],
                    module: "whatever".into(),
                    package: "whatever".into(),
                    name: "Int".into(),
                    publicity: Publicity::Public,
                }),
            })),
        },
        "Int",
    );
    assert_string!(
        Type::Var {
            type_: Arc::new(RefCell::new(TypeVar::Unbound { id: 2231 })),
        },
        "a",
    );
    assert_string!(
        fn_(
            vec![Arc::new(Type::Var {
                type_: Arc::new(RefCell::new(TypeVar::Unbound { id: 78 })),
            })],
            Arc::new(Type::Var {
                type_: Arc::new(RefCell::new(TypeVar::Unbound { id: 2 })),
            }),
        ),
        "fn(a) -> b",
    );
    assert_string!(
        fn_(
            vec![Arc::new(Type::Var {
                type_: Arc::new(RefCell::new(TypeVar::Generic { id: 78 })),
            })],
            Arc::new(Type::Var {
                type_: Arc::new(RefCell::new(TypeVar::Generic { id: 2 })),
            }),
        ),
        "fn(a) -> b",
    );
}

#[test]
fn function_test() {
    assert_eq!(pretty_print(fn_(vec![], int())), "fn() -> Int");

    assert_eq!(
        pretty_print(fn_(vec![int(), int(), int()], int())),
        "fn(Int, Int, Int) -> Int"
    );

    assert_eq!(
        pretty_print(fn_(
            vec![
                float(),
                float(),
                float(),
                float(),
                float(),
                float(),
                float(),
                float(),
                float(),
                float(),
                float(),
                float(),
                float()
            ],
            float()
        )),
        "fn(
  Float,
  Float,
  Float,
  Float,
  Float,
  Float,
  Float,
  Float,
  Float,
  Float,
  Float,
  Float,
  Float,
) -> Float"
    );

    assert_eq!(
        pretty_print(fn_(
            vec![
                tuple(vec![float(), float(), float(), float(), float(), float()]),
                float(),
                float(),
                float(),
                float(),
                float(),
                float(),
                float()
            ],
            float()
        )),
        "fn(
  #(Float, Float, Float, Float, Float, Float),
  Float,
  Float,
  Float,
  Float,
  Float,
  Float,
  Float,
) -> Float"
    );

    assert_eq!(
        pretty_print(fn_(
            vec![tuple(vec![
                float(),
                float(),
                float(),
                float(),
                float(),
                float()
            ]),],
            tuple(vec![
                tuple(vec![float(), float(), float(), float(), float(), float()]),
                tuple(vec![float(), float(), float(), float(), float(), float()]),
            ]),
        )),
        "fn(#(Float, Float, Float, Float, Float, Float)) ->
  #(
    #(Float, Float, Float, Float, Float, Float),
    #(Float, Float, Float, Float, Float, Float),
  )"
    );
}

/// qualify types that are imported from external modules
/// with a qualified import
/// ```gleam
/// import external_module
/// external_module.MyType
/// ```
#[test]
fn qualify_external_imported_modules_qualified() {
    let t = Type::Named {
        publicity: Publicity::Public,
        name: "MyType".into(),
        module: "external_module".into(),
        package: "some_package".into(),
        args: vec![],
    };

    let mut printer = Printer::new();
    printer.with_imports_context(
        "my_module".into(),
        vec![Import {
            module: "external_module".into(),
            renaming: Default::default(),
            unqualified_types: Default::default(),
        }],
    );

    assert_eq!(printer.pretty_print(&t, 0), "external_module.MyType")
}

/// qualify types that come from external modules but are not imported
/// ```gleam
/// external_module.MyType
/// ```
#[test]
fn qualify_external_unimported_modules() {
    let t = Type::Named {
        publicity: Publicity::Public,
        name: "MyType".into(),
        module: "external_module".into(),
        package: "some_package".into(),
        args: vec![],
    };

    let mut printer = Printer::new();
    printer.with_imports_context("my_module".into(), vec![]);
    assert_eq!(printer.pretty_print(&t, 0), "external_module.MyType")
}

/// qualify types that come from external modules that are renamed
/// ```gleam
/// import external_module as renamed_module
/// renamed_module.MyType
/// ```
#[test]
fn qualify_external_renamed_modules() {
    let t = Type::Named {
        publicity: Publicity::Public,
        name: "MyType".into(),
        module: "external_module".into(),
        package: "some_package".into(),
        args: vec![],
    };

    let mut printer = Printer::new();
    printer.with_imports_context(
        "my_module".into(),
        vec![Import {
            module: "external_module".into(),
            renaming: Some("renamed_module".into()),
            unqualified_types: Default::default(),
        }],
    );

    assert_eq!(printer.pretty_print(&t, 0), "renamed_module.MyType")
}

/// ```gleam
/// type MyType {}
/// MyType
/// ```
#[test]
fn do_not_qualify_types_defined_in_same_module() {
    let t = Type::Named {
        publicity: Publicity::Public,
        name: "MyType".into(),
        module: "my_module".into(),
        package: "my_package".into(),
        args: vec![],
    };

    let mut printer = Printer::new();
    printer.with_imports_context(
        "my_module".into(),
        vec![Import {
            module: "my_module".into(),
            renaming: Some("renamed_module".into()),
            unqualified_types: Default::default(),
        }],
    );

    assert_eq!(printer.pretty_print(&t, 0), "MyType")
}

#[test]
fn do_not_qualify_gleam_prelude_types() {
    let t = int();

    let mut printer = Printer::new();
    printer.with_imports_context("my_module".into(), vec![]);
    assert_eq!(printer.pretty_print(&t, 0), "Int")
}

/// ```gleam
/// import external_module.{type MyType}
/// MyType
/// ```
#[test]
fn do_not_qualify_types_with_unqualified_imports() {
    let t = Type::Named {
        publicity: Publicity::Public,
        name: "MyType".into(),
        module: "external_module".into(),
        package: "some_package".into(),
        args: vec![],
    };

    let mut printer = Printer::new();
    printer.with_imports_context(
        "my_module".into(),
        vec![Import {
            module: "external_module".into(),
            renaming: None,
            unqualified_types: vec![UnqualifiedImport {
                name: "MyType".into(),
                as_name: None,
            }],
        }],
    );
    assert_eq!(printer.pretty_print(&t, 0), "MyType")
}

/// ```gleam
/// import external_module.{type MyType as RenamedType}
/// RenamedType
/// ```
#[test]
fn do_not_qualify_types_with_unqualified_imports_and_rename() {
    let t = Type::Named {
        publicity: Publicity::Public,
        name: "MyType".into(),
        module: "external_module".into(),
        package: "some_package".into(),
        args: vec![],
    };

    let mut printer = Printer::new();
    printer.with_imports_context(
        "my_module".into(),
        vec![Import {
            module: "external_module".into(),
            renaming: None,
            unqualified_types: vec![UnqualifiedImport {
                name: "MyType".into(),
                as_name: Some("RenamedType".into()),
            }],
        }],
    );
    assert_eq!(printer.pretty_print(&t, 0), "RenamedType")
}

#[cfg(test)]
fn pretty_print(type_: Arc<Type>) -> String {
    Printer::new().pretty_print(&type_, 0)
}
