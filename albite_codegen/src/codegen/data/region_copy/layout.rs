//! `copy_layout`: a Tungsten type → the shape its lowered value has in
//! memory, as the runtime's copy-out reads it (ADR 13.10.26q D4).
//!
//! Pure: no LLVM. It mirrors `TypeLowering::lower_type`'s rules — `Nat`/`Int`
//! an `i64`, `Bool` an `i1`, `String` a `{ ptr, i64 }`, a product a struct, a
//! sum or a flat ADT the `{ i32, [N x i8] }` blob with its payload at byte 4,
//! a μ-type or recursive ADT a pointer to a block holding its unfolding — and
//! computes sizes and offsets by the C layout rules both link targets share.
//! A `compile/` test per shape checks them against LLVM's `TargetData`.
//!
//! **Mutual clusters.** A μ-encoding wraps only the cluster's entry member, so
//! inside `μα_Tree. μα_Forest. (Nat × α_Forest)` the binder `α_Forest` is a
//! *marker* for the type named `Forest`, not the body it binds. Read closed,
//! a Tree's second field would be laid out as a Tree. So only a chain's first
//! binder is bound to the μ-type itself; every other binder, and a free
//! `α_Name`, resolves by name through the ADT table, as `is_recursive_adt`'s
//! `α_{name}` convention does.

use std::collections::HashMap;

use albite_core::types::Type;

use super::type_names::collect_names;

/// The named types a layout resolves through — `TypeLowering`'s tables in
/// the code generator, a map in the tests.
pub trait NamedTypes {
    /// A record's encoding as nested products.
    fn record(&self, name: &str) -> Option<Type>;
    /// An ADT's constructor payloads with `args` substituted for its
    /// parameters, in constructor order; `None` when `name` is no ADT.
    fn adt_payloads(&self, name: &str, args: &[Type]) -> Option<Vec<Type>>;
}

/// Why a type has no copy layout: D11's "a first-order shape the copy cannot
/// classify", which keeps a refusal naming the shape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unclassified {
    /// The part of the type the layout could not describe.
    pub shape: String,
}

/// A node of the layout graph, addressed by index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CopyNode {
    /// `{ ptr, i64 }`.
    String,
    /// A by-value struct: `(offset, node)` for each pointer-holding field.
    Struct(Vec<(u64, usize)>),
    /// `{ i32, [N x i8] }`: per variant, its payload size and its node when
    /// it holds a pointer. The payload starts at [`BLOB_PAYLOAD_OFFSET`].
    Tagged(Vec<(u64, Option<usize>)>),
    /// A pointer to a block of `size` bytes described by `content`.
    Box {
        /// The block's size, as `fold_to_heap` allocates it.
        size: u64,
        /// The block's node when it holds a pointer.
        content: Option<usize>,
    },
}

/// The tag is an `i32`, and `[N x i8]` has alignment 1.
pub const BLOB_PAYLOAD_OFFSET: u64 = 4;

/// A type's copy layout: the node graph and the root's place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CopyLayout {
    /// Every node; [`CopyNode::Box`] and the others refer to each other by
    /// index.
    pub nodes: Vec<CopyNode>,
    /// The root's node, or `None` for a value that holds no pointer, which
    /// needs no copy at all.
    pub root: Option<usize>,
    /// The root value's size in bytes, for the code generator's check
    /// against the lowered value.
    pub size: u64,
}

/// Where a lowered value sits: size, alignment, and its node when it holds a
/// pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Placed {
    size: u64,
    align: u64,
    node: Option<usize>,
}

const POINTER: (u64, u64) = (8, 8);

impl Placed {
    const fn scalar(size: u64, align: u64) -> Self {
        Self {
            size,
            align,
            node: None,
        }
    }
}

fn align_up(value: u64, align: u64) -> u64 {
    value.div_ceil(align) * align
}

/// The copy layout of `ty`.
///
/// # Errors
///
/// [`Unclassified`] for a shape the copy cannot describe — a function, a
/// reference, an unresolved type variable, or a marker naming no ADT.
pub fn copy_layout(ty: &Type, named: &impl NamedTypes) -> Result<CopyLayout, Unclassified> {
    let mut builder = Builder {
        named,
        nodes: Vec::new(),
        bound: Vec::new(),
        mu_boxes: Vec::new(),
        named_boxes: Vec::new(),
        recursive: HashMap::new(),
    };
    let root = builder.place(ty)?;
    Ok(CopyLayout {
        nodes: builder.nodes,
        root: root.node,
        size: root.size,
    })
}

struct Builder<'a, N> {
    named: &'a N,
    nodes: Vec<CopyNode>,
    /// μ binders in scope, innermost last: name → its box.
    bound: Vec<(String, usize)>,
    /// μ-types already given a box, so a type nested twice shares one.
    mu_boxes: Vec<(Type, usize)>,
    /// Recursive named types already given a box, by name and arguments.
    named_boxes: Vec<((String, Vec<Type>), usize)>,
    /// Whether a named ADT reaches itself through its payloads.
    recursive: HashMap<String, bool>,
}

impl<N: NamedTypes> Builder<'_, N> {
    fn push(&mut self, node: CopyNode) -> usize {
        self.nodes.push(node);
        self.nodes.len() - 1
    }

    fn unclassified(ty: &Type) -> Unclassified {
        Unclassified {
            shape: ty.to_string(),
        }
    }

    fn place(&mut self, ty: &Type) -> Result<Placed, Unclassified> {
        match ty {
            Type::Bool => Ok(Placed::scalar(1, 1)),
            Type::Nat | Type::Int => Ok(Placed::scalar(8, 8)),
            Type::Unit | Type::Void | Type::Eq(..) => Ok(Placed::scalar(0, 1)),
            Type::String => Ok(Placed {
                size: 16,
                align: 8,
                node: Some(self.push(CopyNode::String)),
            }),
            Type::Product(left, right) | Type::Sigma(_, left, right) => {
                self.place_struct(&[left, right])
            }
            Type::Sum(left, right) => self.place_tagged(&[(**left).clone(), (**right).clone()]),
            Type::Adt(_, _, variants) => {
                let payloads: Vec<Type> = variants.iter().map(|(_, ty)| ty.clone()).collect();
                self.place_tagged(&payloads)
            }
            Type::Forall(_, body) => self.place(body),
            Type::Mu(..) => self.place_mu(ty),
            Type::TyVar(name) => self.place_named_var(name, ty),
            Type::App(name, args) => self.place_named(name, args, ty),
            Type::PredApp(..) | Type::ForallPred(..) | Type::PredAbs(..) => {
                self.place(&ty.erase_dependency())
            }
            Type::Arrow(..) | Type::Pi(..) | Type::Ptr(_) | Type::Ref(_) | Type::Error => {
                Err(Self::unclassified(ty))
            }
        }
    }

    fn place_struct(&mut self, fields: &[&Type]) -> Result<Placed, Unclassified> {
        let (mut offset, mut align) = (0, 1);
        let mut pointers = Vec::new();
        for field in fields {
            let placed = self.place(field)?;
            let at = align_up(offset, placed.align);
            if let Some(node) = placed.node {
                pointers.push((at, node));
            }
            offset = at + placed.size;
            align = align.max(placed.align);
        }
        let node = (!pointers.is_empty()).then(|| self.push(CopyNode::Struct(pointers)));
        Ok(Placed {
            size: align_up(offset, align),
            align,
            node,
        })
    }

    /// The `{ i32, [N x i8] }` blob, `N` the largest payload's size.
    fn place_tagged(&mut self, payloads: &[Type]) -> Result<Placed, Unclassified> {
        let mut variants = Vec::with_capacity(payloads.len());
        for payload in payloads {
            let placed = self.place(payload)?;
            variants.push((placed.size, placed.node));
        }
        let largest = variants.iter().map(|(size, _)| *size).max().unwrap_or(0);
        let node = variants
            .iter()
            .any(|(_, node)| node.is_some())
            .then(|| self.push(CopyNode::Tagged(variants)));
        Ok(Placed {
            size: align_up(BLOB_PAYLOAD_OFFSET + largest, 4),
            align: 4,
            node,
        })
    }

    /// A pointer to a fresh box whose block `fill` describes.
    fn place_box(
        &mut self,
        fill: impl FnOnce(&mut Self, usize) -> Result<Placed, Unclassified>,
    ) -> Result<(Placed, usize), Unclassified> {
        let id = self.push(CopyNode::Box {
            size: 0,
            content: None,
        });
        let block = fill(self, id)?;
        self.nodes[id] = CopyNode::Box {
            size: block.size,
            content: block.node,
        };
        Ok((Self::pointer_to(id), id))
    }

    const fn pointer_to(id: usize) -> Placed {
        Placed {
            size: POINTER.0,
            align: POINTER.1,
            node: Some(id),
        }
    }

    fn place_mu(&mut self, ty: &Type) -> Result<Placed, Unclassified> {
        let mu_boxes: &[(Type, usize)] = &self.mu_boxes;
        if let Some((_, id)) = mu_boxes.iter().find(|(seen, _)| seen == ty) {
            return Ok(Self::pointer_to(*id));
        }
        let mut chain = Vec::new();
        let mut body = ty;
        while let Type::Mu(var, inner) = body {
            chain.push(var.clone());
            body = inner;
        }
        let (placed, _) = self.place_box(|builder, id| {
            builder.mu_boxes.push((ty.clone(), id));
            let scope = builder.bound.len();
            builder.bound.push((chain[0].clone(), id));
            // Every other binder of the chain is a marker for its member.
            for marker in &chain[1..] {
                let member = builder.place_marker(marker, ty)?;
                let member = member.node.expect("a marker places a pointer");
                builder.bound.push((marker.clone(), member));
            }
            let block = builder.place(body);
            builder.bound.truncate(scope);
            block
        })?;
        Ok(placed)
    }

    /// `α_Name` free of every binder: a pointer to the named member.
    fn place_marker(&mut self, marker: &str, whole: &Type) -> Result<Placed, Unclassified> {
        let name = marker
            .strip_prefix("α_")
            .ok_or_else(|| Self::unclassified(whole))?;
        self.named_pointer(name, &[], whole)
    }

    fn place_named_var(&mut self, name: &str, ty: &Type) -> Result<Placed, Unclassified> {
        let in_scope: &[(String, usize)] = &self.bound;
        if let Some((_, id)) = in_scope.iter().rev().find(|(bound, _)| bound == name) {
            return Ok(Self::pointer_to(*id));
        }
        if name.starts_with("α_") {
            return self.place_marker(name, ty);
        }
        let name = name.strip_prefix('@').unwrap_or(name);
        if let Some(record) = self.named.record(name) {
            return self.place(&record);
        }
        self.place_named(name, &[], ty)
    }

    /// A named ADT applied to `args`: a pointer when it is recursive, its
    /// payloads inline otherwise.
    fn place_named(
        &mut self,
        name: &str,
        args: &[Type],
        ty: &Type,
    ) -> Result<Placed, Unclassified> {
        if self.is_recursive(name) {
            return self.named_pointer(name, args, ty);
        }
        let payloads = self
            .named
            .adt_payloads(name, args)
            .ok_or_else(|| Self::unclassified(ty))?;
        self.place_payloads(&payloads)
    }

    /// One constructor is its bare payload; more are the blob.
    fn place_payloads(&mut self, payloads: &[Type]) -> Result<Placed, Unclassified> {
        match payloads {
            [] => Ok(Placed::scalar(0, 1)),
            [only] => self.place(only),
            _ => self.place_tagged(payloads),
        }
    }

    fn named_pointer(
        &mut self,
        name: &str,
        args: &[Type],
        ty: &Type,
    ) -> Result<Placed, Unclassified> {
        let key = (name.to_string(), args.to_vec());
        let named_boxes: &[((String, Vec<Type>), usize)] = &self.named_boxes;
        if let Some((_, id)) = named_boxes.iter().find(|(seen, _)| *seen == key) {
            return Ok(Self::pointer_to(*id));
        }
        let payloads = self
            .named
            .adt_payloads(name, args)
            .ok_or_else(|| Self::unclassified(ty))?;
        let (placed, _) = self.place_box(|builder, id| {
            builder.named_boxes.push((key, id));
            builder.place_payloads(&payloads)
        })?;
        Ok(placed)
    }

    /// Whether `name` reaches itself through its constructors' fields — a
    /// member of a recursive cluster, which the lowering boxes.
    fn is_recursive(&mut self, name: &str) -> bool {
        if let Some(known) = self.recursive.get(name) {
            return *known;
        }
        let mut seen = Vec::new();
        let mut pending = self.referenced_names(name);
        let mut found = false;
        while let Some(next) = pending.pop() {
            if next == name {
                found = true;
                break;
            }
            if !seen.contains(&next) {
                pending.extend(self.referenced_names(&next));
                seen.push(next);
            }
        }
        self.recursive.insert(name.to_string(), found);
        found
    }

    /// The type names `name`'s payloads mention, with `@` and `α_` stripped.
    fn referenced_names(&self, name: &str) -> Vec<String> {
        let mut names = Vec::new();
        for payload in self.named.adt_payloads(name, &[]).unwrap_or_default() {
            collect_names(&payload, &mut names);
        }
        names
    }
}
