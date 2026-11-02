//! The tree `html5ever` builds, held in a flat arena.
//!
//! A node names its parent and its children by index, so dropping a tree of
//! any depth frees one vector and never recurses. `markup5ever_rcdom` holds
//! its nodes behind `Rc`, and its authors publish it as unsupported and
//! test-only.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;

use html5ever::interface::{ElementFlags, NodeOrText, QuirksMode, TreeSink};
use html5ever::tendril::StrTendril;
use html5ever::{Attribute, QualName};

use ambre_core::index::{Axis, IdxVec, Pos};

/// What a node of the arena holds. A comment, a doctype and a processing
/// instruction carry nothing a paste reads.
pub(super) enum Data {
    Document,
    Element {
        name: QualName,
        attrs: Vec<Attribute>,
    },
    Text(String),
    Ignored,
}

pub(super) struct Node {
    pub(super) parent: Option<Pos<HtmlNode>>,
    pub(super) children: Vec<Pos<HtmlNode>>,
    pub(super) data: Data,
}

impl Node {
    /// A node under no parent, holding `data`.
    const fn orphan(data: Data) -> Self {
        Self {
            parent: None,
            children: Vec::new(),
            data,
        }
    }
}

/// The places of the arena: a [`Pos<HtmlNode>`] names a node. Only the
/// arena hands one out, so a position always names a node of the tree it
/// came from.
#[derive(PartialEq, Eq)]
pub(super) enum HtmlNode {}

impl Axis for HtmlNode {
    const NAME: &'static str = "HtmlNode";
}

/// The document, the root every other node hangs from.
pub(super) const DOCUMENT: Pos<HtmlNode> = Pos::ZERO;

/// A node's index, and the element name the tree builder asks for without
/// reaching into the arena.
#[derive(Clone)]
pub(super) struct Handle {
    id: Pos<HtmlNode>,
    name: QualName,
}

/// The arena the tree builder writes into. Index 0 is the document.
pub(super) struct Arena {
    nodes: RefCell<IdxVec<HtmlNode, Node>>,
    /// A template's contents: a fragment no parent holds, so a paste never
    /// reads it.
    templates: RefCell<HashMap<Pos<HtmlNode>, Pos<HtmlNode>>>,
}

fn nameless() -> QualName {
    QualName::new(None, html5ever::ns!(), html5ever::local_name!(""))
}

impl Arena {
    pub(super) fn new() -> Self {
        Self {
            nodes: RefCell::new(IdxVec::from(vec![Node::orphan(Data::Document)])),
            templates: RefCell::new(HashMap::new()),
        }
    }

    fn push(&self, data: Data) -> Pos<HtmlNode> {
        self.nodes.borrow_mut().push(Node::orphan(data))
    }

    fn handle(&self, data: Data) -> Handle {
        let name = match &data {
            Data::Element { name, .. } => name.clone(),
            _ => nameless(),
        };
        Handle {
            id: self.push(data),
            name,
        }
    }

    fn detach(nodes: &mut IdxVec<HtmlNode, Node>, id: Pos<HtmlNode>) {
        let Some(parent) = nodes.get(id).and_then(|node| node.parent) else {
            return;
        };
        if let Some(parent) = nodes.get_mut(parent) {
            parent.children.retain(|child| *child != id);
        }
        if let Some(node) = nodes.get_mut(id) {
            node.parent = None;
        }
    }

    /// Put `child` under `parent` at `index` among its children, merging
    /// text into a text neighbour as the tree builder expects.
    fn insert(&self, parent: Pos<HtmlNode>, index: usize, child: NodeOrText<Handle>) {
        let mut nodes = self.nodes.borrow_mut();
        let Some(count) = nodes.get(parent).map(|node| node.children.len()) else {
            return;
        };
        let index = index.min(count);
        let id = match child {
            NodeOrText::AppendText(text) => {
                let before = index
                    .checked_sub(1)
                    .and_then(|i| nodes.get(parent)?.children.get(i).copied());
                if let Some(Node {
                    data: Data::Text(previous),
                    ..
                }) = before.and_then(|b| nodes.get_mut(b))
                {
                    previous.push_str(&text);
                    return;
                }
                nodes.push(Node::orphan(Data::Text(text.to_string())))
            }
            NodeOrText::AppendNode(handle) => {
                Self::detach(&mut nodes, handle.id);
                handle.id
            }
        };
        // Detaching may have shifted the siblings.
        let Some(parent_node) = nodes.get_mut(parent) else {
            return;
        };
        let index = index.min(parent_node.children.len());
        parent_node.children.insert(index, id);
        if let Some(node) = nodes.get_mut(id) {
            node.parent = Some(parent);
        }
    }

    fn append_to(&self, parent: Pos<HtmlNode>, child: NodeOrText<Handle>) {
        let end = self
            .nodes
            .borrow()
            .get(parent)
            .map_or(0, |node| node.children.len());
        self.insert(parent, end, child);
    }
}

impl TreeSink for Arena {
    type Handle = Handle;
    type Output = IdxVec<HtmlNode, Node>;
    type ElemName<'a> = &'a QualName;

    fn finish(self) -> IdxVec<HtmlNode, Node> {
        self.nodes.into_inner()
    }

    fn parse_error(&self, _msg: Cow<'static, str>) {}

    fn get_document(&self) -> Handle {
        Handle {
            id: DOCUMENT,
            name: nameless(),
        }
    }

    fn elem_name<'a>(&'a self, target: &'a Handle) -> &'a QualName {
        &target.name
    }

    fn create_element(&self, name: QualName, attrs: Vec<Attribute>, flags: ElementFlags) -> Handle {
        let handle = self.handle(Data::Element { name, attrs });
        if flags.template {
            let contents = self.push(Data::Ignored);
            self.templates.borrow_mut().insert(handle.id, contents);
        }
        handle
    }

    fn create_comment(&self, _text: StrTendril) -> Handle {
        self.handle(Data::Ignored)
    }

    fn create_pi(&self, _target: StrTendril, _data: StrTendril) -> Handle {
        self.handle(Data::Ignored)
    }

    fn append(&self, parent: &Handle, child: NodeOrText<Handle>) {
        self.append_to(parent.id, child);
    }

    fn append_based_on_parent_node(
        &self,
        element: &Handle,
        prev_element: &Handle,
        child: NodeOrText<Handle>,
    ) {
        let has_parent = self
            .nodes
            .borrow()
            .get(element.id)
            .is_some_and(|node| node.parent.is_some());
        if has_parent {
            self.append_before_sibling(element, child);
        } else {
            self.append_to(prev_element.id, child);
        }
    }

    fn append_doctype_to_document(&self, _: StrTendril, _: StrTendril, _: StrTendril) {}

    fn get_template_contents(&self, target: &Handle) -> Handle {
        let id = self.templates.borrow().get(&target.id).copied();
        match id {
            Some(id) => Handle {
                id,
                name: nameless(),
            },
            None => self.handle(Data::Ignored),
        }
    }

    fn same_node(&self, x: &Handle, y: &Handle) -> bool {
        x.id == y.id
    }

    fn set_quirks_mode(&self, _mode: QuirksMode) {}

    fn append_before_sibling(&self, sibling: &Handle, new_node: NodeOrText<Handle>) {
        let place = {
            let nodes = self.nodes.borrow();
            nodes
                .get(sibling.id)
                .and_then(|node| node.parent)
                .and_then(|parent| {
                    let index = nodes
                        .get(parent)?
                        .children
                        .iter()
                        .position(|child| *child == sibling.id)?;
                    Some((parent, index))
                })
        };
        if let Some((parent, index)) = place {
            self.insert(parent, index, new_node);
        }
    }

    fn add_attrs_if_missing(&self, target: &Handle, attrs: Vec<Attribute>) {
        let mut nodes = self.nodes.borrow_mut();
        if let Some(Node {
            data: Data::Element { attrs: held, .. },
            ..
        }) = nodes.get_mut(target.id)
        {
            for attr in attrs {
                if !held.iter().any(|a| a.name == attr.name) {
                    held.push(attr);
                }
            }
        }
    }

    fn remove_from_parent(&self, target: &Handle) {
        Self::detach(&mut self.nodes.borrow_mut(), target.id);
    }

    fn reparent_children(&self, node: &Handle, new_parent: &Handle) {
        let mut nodes = self.nodes.borrow_mut();
        let Some(children) = nodes
            .get_mut(node.id)
            .map(|n| std::mem::take(&mut n.children))
        else {
            return;
        };
        for child in &children {
            if let Some(child) = nodes.get_mut(*child) {
                child.parent = Some(new_parent.id);
            }
        }
        if let Some(parent) = nodes.get_mut(new_parent.id) {
            parent.children.extend(children);
        }
    }
}
