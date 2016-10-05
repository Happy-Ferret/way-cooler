//! Main module to handle the layout.
//! This is where the i3-specific code is.

use std::ops::Deref;
use petgraph::graph::NodeIndex;
use uuid::Uuid;
use rustwlc::{WlcView, WlcOutput};
use super::super::LayoutTree;
use super::container::{Container, ContainerType};
use super::super::actions::focus::FocusError;
use super::super::actions::movement::MovementError;
use super::super::actions::layout::LayoutErr;


use super::super::core::graph_tree::GraphError;

use super::super::commands::CommandResult;

#[derive(Clone, Copy, Debug)]
pub enum Direction {
    Up,
    Down,
    Right,
    Left
}

#[derive(Clone, Debug)]
pub enum TreeError {
    /// A Node can not be found in the tree with this Node Handle.
    NodeNotFound(Uuid),
    /// The node was removed from the tree already.
    /// Depending on this operation this can be ignored, or should
    /// raise some flags that something is up
    NodeWasRemoved(NodeIndex),
    /// A WlcView handle could not be found in the tree.
    ViewNotFound(WlcView),
    /// A UUID was not associated with the this type of container.
    UuidNotAssociatedWith(ContainerType),
    /// UUID was associated with wrong container type,
    /// expected a container that had one of those other types.
    UuidWrongType(Uuid, Vec<ContainerType>),
    /// There was no active/focused container.
    NoActiveContainer,
    /// A container was the root container, which was not expected
    InvalidOperationOnRootContainer(Uuid),
    /// There was an error in the graph, an invariant of one of the
    /// functions were not held, so this might be an issue in the Tree.
    PetGraph(GraphError),
    /// An error occurred while trying to focus on a container
    Focus(FocusError),
    /// An error occurred while trying to move a container
    Movement(MovementError),
    /// An error occurred while trying to change the layout
    Layout(LayoutErr),
    /// The tree was (true) or was not (false) performing an action,
    /// but the opposite value was expected.
    PerformingAction(bool)
}

impl LayoutTree {
    /// Drops every node in the tree, essentially invalidating it
    pub fn destroy_tree(&mut self) {
        let root_ix = self.tree.root_ix();
        let mut nodes = self.tree.all_descendants_of(root_ix);
        nodes.sort_by(|a, b| b.cmp(a));
        for node in nodes {
            self.tree.remove(node);
        }
        self.unset_active_container();
    }

    /// Sets the active container by finding the node with the WlcView
    pub fn set_active_view(&mut self, handle: WlcView) -> CommandResult {
        if let Some(node_ix) = self.tree.descendant_with_handle(self.tree.root_ix(), &handle) {
            self.set_active_node(node_ix)
        } else {
            Err(TreeError::ViewNotFound(handle))
        }
    }

    /// Sets the active container associated with the UUID to be the active container.
    ///
    /// Updates the active path accordingly
    ///
    /// If the container was not a view or container, or the UUID was invalid,
    /// then an error is returned.
    pub fn set_active_container(&mut self, id: Uuid) -> CommandResult {
        let node_ix = try!(self.tree.lookup_id(id)
                           .ok_or(TreeError::NodeNotFound(id)));
        self.set_active_node(node_ix)
    }

    /// Looks up the id, returning the container associated with it.
    pub fn lookup(&self, id: Uuid) -> Result<&Container, TreeError> {
        self.tree.lookup_id(id)
            .map(|node_ix| &self.tree[node_ix])
            .ok_or(TreeError::NodeNotFound(id))
    }

    /// Sets the active container to be the given node.
    pub fn set_active_node(&mut self, node_ix: NodeIndex) -> CommandResult {
        info!("Active container was {:?}", self.active_container);
        self.active_container = Some(node_ix);
        match self.tree[node_ix] {
            Container::View { ref handle, .. } => handle.focus(),
            Container::Container { .. } => {},
            ref container => return Err(
                TreeError::UuidWrongType(container.get_id(),
                                         vec!(ContainerType::View, ContainerType::Container)))
        }
        if !self.tree[node_ix].floating() {
            self.tree.set_ancestor_paths_active(node_ix);
        }
        info!("Active container is now: {:?}", self.active_container);
        Ok(())
    }

    /// Unsets the active container. This should be used when focusing on
    /// a view that is not a part of the tree.
    pub fn unset_active_container(&mut self) {
        self.active_container = None;
    }

    /// Gets the root container of the active container.
    ///
    /// If there is no active container, searches the path.
    pub fn root_container_ix(&self) -> Option<NodeIndex> {
        if let Some(active_ix) = self.active_container {
            let mut cur_ix = active_ix;
            while let Ok(parent_ix) = self.tree.parent_of(cur_ix) {
                if self.tree[cur_ix].get_type() == ContainerType::Container
                    && self.tree[parent_ix].get_type() == ContainerType::Workspace {
                        return Some(cur_ix)
                    } else {
                    cur_ix = parent_ix;
                }
            }
            None
        } else {
            // Check the path
            let root_ix = self.tree.root_ix();
            self.tree.follow_path_until(root_ix, ContainerType::Container).ok()
        }
    }

    /// Gets the currently active container.
    pub fn get_active_container(&self) -> Option<&Container> {
        self.active_container.and_then(|ix| self.tree.get(ix))
    }

    /// Gets the currently active container.
    pub fn get_active_container_mut(&mut self) -> Option<&mut Container> {
        self.active_container.and_then(move |ix| self.tree.get_mut(ix))
    }

    /// Gets the index of the currently active container with the given type.
    /// Starts at the active container, moves up until either a container with
    /// that type is found or the root node is hit
    pub fn active_ix_of(&self, ctype: ContainerType) -> Option<NodeIndex> {
        if let Some(ix) = self.active_container {
            if self.tree[ix].get_type() == ctype {
                Some(ix)
            } else {
                self.tree.ancestor_of_type(ix, ctype).ok()
            }
        } else {
            None
        }

    }

    /// Determines if the active container is the root container
    pub fn active_is_root(&self) -> bool {
        if let Some(active_ix) = self.active_container {
            self.tree.is_root_container(active_ix)
        } else {
            false
        }
    }

    /// Add a new view container with the given WlcView to the active container
    pub fn add_view(&mut self, view: WlcView) -> Result<Uuid, TreeError> {
        if let Some(mut active_ix) = self.active_container {
            let parent_ix = self.tree.parent_of(active_ix)
                .expect("Active container had no parent");
            // Get the previous position before correcting the container
            let prev_pos = (*self.tree.get_edge_weight_between(parent_ix, active_ix)
                .expect("Could not get edge weight between active and active parent")).deref()
                + 1;
            if self.tree[active_ix].get_type() == ContainerType::View {
                active_ix = try!(self.tree.parent_of(active_ix)
                                 .map_err(|err| TreeError::PetGraph(err)));
            }
            let container = Container::new_view(view);
            let uuid = container.get_id();
            let view_ix = self.tree.add_child(active_ix,
                                              container,
                                              true);
            self.tree.set_child_pos(view_ix, prev_pos);
            self.validate();
            try!(self.set_active_node(view_ix));
            return Ok(uuid)
        }
        self.validate();
        Err(TreeError::NoActiveContainer)
    }

    /// Adds the container with the node index as a child.
    /// The node at the node index is removed and
    /// made a child of the new container node.
    ///
    /// The new container has the same edge weight as the child that is passed in.
    pub fn add_container(&mut self, container: Container, child_ix: NodeIndex) -> CommandResult {
        let parent_ix = self.tree.parent_of(child_ix)
            .expect("Node had no parent");
        let old_weight = *self.tree.get_edge_weight_between(parent_ix, child_ix)
            .expect("parent and children were not connected");
        let new_container_ix = self.tree.add_child(parent_ix, container, false);
        self.tree.move_node(child_ix, new_container_ix);
        self.tree.set_child_pos(new_container_ix, *old_weight);
        try!(self.set_active_node(new_container_ix));
        self.validate();
        Ok(())
    }

    /// Make a new output container with the given WlcOutput.
    ///
    /// A new workspace is automatically added to the output, to ensure
    /// consistency with the tree. By default, it sets this new workspace to
    /// be workspace "1". This will later change to be the first available
    /// workspace if using i3-style workspaces.
    pub fn add_output(&mut self, output: WlcOutput) {
        trace!("Adding new output with {:?}", output);
        let root_index = self.tree.root_ix();
        let output_ix = self.tree.add_child(root_index,
                                            Container::new_output(output),
                                            true);
        self.active_container = Some(self.init_workspace("1".to_string(), output_ix));
        self.validate();
    }

    //// Remove a view container from the tree
    pub fn remove_view(&mut self, view: &WlcView) -> Result<Container, TreeError> {
        if let Some(view_ix) = self.tree.descendant_with_handle(self.tree.root_ix(), view) {
            let container = self.remove_view_or_container(view_ix)
                .expect("Could not remove node we just verified exists!");
            self.validate();
            Ok(container)
        } else {
            self.validate();
            Err(TreeError::ViewNotFound(view.clone()))
        }
    }

    /// Remove a container from the tree.
    /// The active container is preserved after this operation,
    /// if it was moved then it's new index will be reflected in the structure
    ///
    /// Note that because this causes N indices to be changed (where N is the
    /// number of descendants of the container), any node indices should be
    /// considered invalid after this operation (except for the active_container)
    pub fn remove_container(&mut self, container_ix: NodeIndex) -> CommandResult {
        let mut children = self.tree.all_descendants_of(container_ix);
        // add current container to the list as well
        children.push(container_ix);
        for node_ix in children {
            trace!("Removing index {:?}: {:?}", node_ix, self.tree[node_ix]);
            match self.tree.get(node_ix) {
                None => return Err(TreeError::NodeWasRemoved(container_ix)),
                Some(&Container::View { .. }) | Some(&Container::Container { .. }) => {
                    try!(self.remove_view_or_container(node_ix));
                },
                Some(_) => {
                    try!(self.tree.remove(node_ix)
                         .ok_or(TreeError::NodeWasRemoved(container_ix)));
                },
            }
        }
        self.validate();
        Ok(())
    }

    /// Special code to handle removing a View or Container.
    /// We have to ensure that we aren't invalidating the active container
    /// when we remove a view or container.
    pub fn remove_view_or_container(&mut self, node_ix: NodeIndex) -> Result<Container, TreeError> {
        // Only the root container has a non-container parent, and we can't remove that
        let c_type: ContainerType;
        let uuid: Uuid;
        {
            let container = &try!(self.tree.get(node_ix).ok_or(TreeError::NodeWasRemoved(node_ix)));
            c_type = container.get_type();
            uuid = container.get_id();
        }
        if c_type != ContainerType::View && c_type != ContainerType::Container {
            return Err(TreeError::UuidWrongType(uuid, vec!(ContainerType::View, ContainerType::Container)));
        }
        let parent_ix = self.tree.ancestor_of_type(node_ix, ContainerType::Container)
            .unwrap_or_else(|_| self.tree.ancestor_of_type(node_ix, ContainerType::Workspace)
            .expect("No idea where the node is, are you sure the tree is valid?"));
        let container = try!(self.tree.remove(node_ix)
                                .ok_or(TreeError::NodeWasRemoved(node_ix)));
        match container {
            Container::View { .. } | Container::Container { .. } => {},
            _ => unreachable!()
        };
        let result = Ok(container);
        self.focus_on_next_container(parent_ix);
        // Remove parent container if it is a non-root container and has no other children
        let parent_type = self.tree[parent_ix].get_type();
        match parent_type {
            ContainerType::Container => {
                if self.tree.can_remove_empty_parent(parent_ix) {
                    try!(self.remove_view_or_container(parent_ix));
                }
                self.validate();
            }
            _ => {},
        }
        trace!("Removed container {:?}, index {:?}", result, node_ix);
        result
    }

    /// Removes the current active container
    pub fn remove_active(&mut self) -> Result<Container, TreeError> {
        if let Some(active_ix) = self.active_container {
            self.remove_view_or_container(active_ix)
        } else {
            Err(TreeError::NoActiveContainer)
        }
    }

    /// Validates the tree
    #[cfg(debug_assertions)]
    pub fn validate(&self) {
        // Recursive method to ensure child/parent nodes are connected
        fn validate_node_connections(this: &LayoutTree, parent_ix: NodeIndex) {
            for child_ix in this.tree.children_of(parent_ix) {
                let child_parent = this.tree.parent_of(child_ix)
                    .expect("connections: Child did not point to parent!");
                if child_parent != parent_ix {
                    error!("Child at {:?} has parent {:?}, expected {:?}",
                           child_ix, child_parent, parent_ix);
                    trace!("The tree: {:#?}", this);
                    panic!()
                }
                validate_node_connections(this, child_ix);
            }
        }

        validate_node_connections(self, self.tree.root_ix());

        // Ensure active container is in tree and of right type
        if let Some(active_ix) = self.active_container {
            if self.active_container.is_some() {
                let active = self.get_active_container()
                    .expect("active_container points to invalid node");
                match active.get_type() {
                    ContainerType::View | ContainerType::Container => {},
                    _ => panic!("Active container was not view or container")
                }
                // Check active container in tree
                if self.tree.ancestor_of_type(active_ix, ContainerType::Root).is_err() {
                    error!("Active container @ {:?} is not part of tree!", active_ix);
                    error!("Active container is {:?}", active);
                    trace!("The tree: {:#?}", self);
                    panic!()
                }
            }
        }

        // Ensure workspace have at least one child
        for output_ix in self.tree.children_of(self.tree.root_ix()) {
            for workspace_ix in self.tree.children_of(output_ix) {
                if self.tree.children_of(workspace_ix).len() == 0 {
                    error!("Workspace {:#?} has no children",
                           self.tree[workspace_ix]);
                    trace!("The tree: {:#?}", self);
                    panic!()
                }
                for container_ix in self.tree.all_descendants_of(workspace_ix) {
                    match self.tree[container_ix] {
                        Container::Container { .. } => {
                            let parent_ix = self.tree.parent_of(container_ix)
                                .expect("Container had no parent");
                            if self.tree.children_of(container_ix).len() == 0
                                && self.tree[parent_ix].get_type() != ContainerType::Workspace {
                                    error!("Tree in invalid state. {:?} is an empty non-root container\n \
                                            {:#?}", container_ix, *self);
                                    panic!();
                            }
                            assert!(! self.tree.can_remove_empty_parent(container_ix));
                        },
                        Container::View { .. } => {
                        }
                        _ => panic!("Non-view/container as descendant of a workspace!")
                    }
                }
            }
        }

        // Ensure that edge weights are always monotonically increasing
        fn validate_edge_count(this: &LayoutTree, parent_ix: NodeIndex) {
            // note that the weight should never actually be 0
            let mut cur_weight = 0;
            for child_ix in this.tree.children_of(parent_ix) {
                let weight = *this.tree.get_edge_weight_between(parent_ix, child_ix)
                    .expect("Could not get edge weights between child and parent").deref();
                // Ensure increasing
                if weight <= cur_weight {
                    error!("Weights were not monotonically increasing for children of {:?}", parent_ix);
                    error!("{:#?}", this);
                    panic!("{:?} <= {:?}!", weight, cur_weight);
                }
                // Ensure no holes
                if weight != cur_weight + 1 {
                    error!("Weights have a hole (no child with weight {}) for children of {:?}",
                           cur_weight + 1, parent_ix);
                    error!("{:#?}", this);
                    panic!("Hole in children weights");
                }
                cur_weight = weight;
                validate_edge_count(this, child_ix);
            }
        }
        validate_edge_count(self, self.tree.root_ix());

        // Ensure there is only one active path from the root
        let mut next_ix = Some(self.tree.root_ix());
        while let Some(cur_ix) = next_ix {
            next_ix = None;
            let mut flipped = false;
            for child_ix in self.tree.children_of(cur_ix) {
                let weight = *self.tree.get_edge_weight_between(cur_ix, child_ix)
                    .expect("Could not get edge weights between child and parent");
                if weight.active && flipped {
                    error!("Divergent paths detected!");
                    trace!("Tree: {:#?}", self);
                    panic!("Divergent paths detected!");
                }  else if weight.active {
                    flipped = true;
                }
            }
        }
        let root_ix = self.tree.root_ix();
        for node_ix in self.tree.follow_path_until(root_ix, ContainerType::View) {
            if self.tree[node_ix].floating() {
                error!("{:?} cannot be both on the active path and floating!\n{:#?}",
                       node_ix, self);
                panic!("Found node that was on the active path and floating!");
            }
        }

        // ensure that the active container is valid
        if let Some(node_ix) = self.active_container {
            assert!(self.tree.get(node_ix).is_some());
        }
    }

    #[cfg(not(debug_assertions))]
    pub fn validate(&self) {}
}

#[cfg(test)]
pub mod tests {
    use super::super::super::LayoutTree;
    use super::super::super::core::container::*;
    use super::super::super::core::InnerTree;
    use super::*;
    use rustwlc::*;

    use uuid::Uuid;

    /// Makes a very basic tree.
    /// There is only one output,
    /// Two workspaces,
    /// First workspace has a single view in the root container,
    /// second workspace has a container with two views in it
    /// (the container is a child of the root container).
    ///
    /// The active container is the only view in the first workspace
    #[allow(unused_variables)]
    pub fn basic_tree() -> LayoutTree {
        let mut tree = InnerTree::new();
        let fake_view_1 = WlcView::root();
        let fake_output = fake_view_1.clone().as_output();
        let root_ix = tree.root_ix();
        let fake_size = Size { h: 800, w: 600 };
        let fake_geometry = Geometry {
            size: fake_size.clone(),
            origin: Point { x: 0, y: 0 }
        };

        let output_ix = tree.add_child(root_ix, Container::new_output(fake_output), false);
        let workspace_1_ix = tree.add_child(output_ix,
                                                Container::new_workspace("1".to_string(),
                                                                   fake_size.clone()), false);
        let root_container_1_ix = tree.add_child(workspace_1_ix,
                                                Container::new_container(fake_geometry.clone()), false);
        let workspace_2_ix = tree.add_child(output_ix,
                                                Container::new_workspace("2".to_string(),
                                                                     fake_size.clone()), false);
        let root_container_2_ix = tree.add_child(workspace_2_ix,
                                                Container::new_container(fake_geometry.clone()), false);
        /* Workspace 1 containers */
        let wkspc_1_view = tree.add_child(root_container_1_ix,
                                                Container::new_view(fake_view_1.clone()), false);
        /* Workspace 2 containers */
        let wkspc_2_container = tree.add_child(root_container_2_ix,
                                                Container::new_container(fake_geometry.clone()), false);
        let wkspc_2_sub_view_1 = tree.add_child(wkspc_2_container,
                                                Container::new_view(fake_view_1.clone()), true);
        let wkspc_2_sub_view_2 = tree.add_child(wkspc_2_container,
                                                Container::new_view(fake_view_1.clone()), false);
        let mut layout_tree = LayoutTree {
            tree: tree,
            active_container: None
        };
        let id = layout_tree.tree[wkspc_1_view].get_id();
        layout_tree.set_active_container(id).unwrap();
        layout_tree
    }

    #[test]
    fn destroy_tree_test() {
        let mut tree = basic_tree();
        tree.destroy_tree();
        let root_ix = tree.tree.root_ix();
        assert!(tree.tree.children_of(root_ix).len() == 0);
    }

    #[test]
    /// Ensures that getting the active container always returns either
    /// a view, a container, or nothing.
    fn active_container_tests() {
        let mut simple_tree = basic_tree();
        /* Standard active_container getters */
        {
            let active_container = simple_tree.get_active_container().unwrap();
            let view_ancestor_ix = simple_tree.active_ix_of(ContainerType::View).unwrap();
            assert_eq!(*active_container, simple_tree.tree[view_ancestor_ix]);
            match *active_container {
                Container::View { .. }| Container::Container { .. }=> {},
                _ => panic!("Active container was not a view or container!")
            }
        }
        {
            let active_container_mut = simple_tree.get_active_container_mut().unwrap();
            match *active_container_mut {
                Container::View { .. }| Container::Container { .. }=> {},
            _ => panic!("Active container was not a view or container!")
            }
        }
        /* Active workspace getters */
        {
            let ix = simple_tree.active_ix_of(ContainerType::Workspace).unwrap();
            let active_workspace = &simple_tree.tree[ix];
            let workspace_ancestor_ix = simple_tree.active_ix_of(ContainerType::Workspace).unwrap();
            assert_eq!(*active_workspace, simple_tree.tree[workspace_ancestor_ix]);
            match *active_workspace {
                Container::Workspace { ref name, .. } => {
                    assert_eq!(name.as_str(), "1")
                },
                _ => panic!("get_active_workspace did not return a workspace")
            }
        }
        {
            let ix = simple_tree.active_ix_of(ContainerType::Workspace).unwrap();
            let active_workspace_mut = &mut simple_tree.tree[ix];
            match *active_workspace_mut {
                Container::Workspace { ref name, .. } => {
                assert_eq!(name.as_str(), "1")
            },
            _ => panic!("get_active_workspace did not return a workspace")
            }
        }
        /* Active output getters */
        {
            let ix = simple_tree.active_ix_of(ContainerType::Output).unwrap();
            let active_output = &simple_tree.tree[ix];
            let output_ancestor_ix = simple_tree.active_ix_of(ContainerType::Output).unwrap();
            assert_eq!(*active_output, simple_tree.tree[output_ancestor_ix]);
            match *active_output {
                Container::Output { ref handle, .. } => {
                    assert_eq!(*handle, WlcView::root().as_output());
                }
                _ => panic!("get_active_output did not return an output")
            }
        }
        {
            let ix = simple_tree.active_ix_of(ContainerType::Output).unwrap();
            let active_output_mut = &mut simple_tree.tree[ix];
            match *active_output_mut {
                Container::Output { ref handle, .. } => {
                    assert_eq!(*handle, WlcView::root().as_output());
                }
                _ => panic!("get_active_output did not return an output")
            }
        }
    }

    #[test]
    fn active_container_test() {
        let mut tree = basic_tree();
        tree.active_container = None;
        assert_eq!(tree.get_active_container(), None);
        assert_eq!(tree.active_ix_of(ContainerType::View), None);
        assert_eq!(tree.active_ix_of(ContainerType::Container), None);
        assert_eq!(tree.active_ix_of(ContainerType::Workspace), None);
        assert!(tree.active_ix_of(ContainerType::Output).is_none());
        assert!(tree.active_ix_of(ContainerType::Root).is_none());
        tree.set_active_view(WlcView::root()).unwrap();
        let view_ix = tree.tree.descendant_with_handle(tree.tree.root_ix(), &WlcView::root()).unwrap();
        assert_eq!(tree.active_container, Some(view_ix));
        tree.unset_active_container();
        assert_eq!(tree.get_active_container(), None);
        assert_eq!(tree.active_container, None);
    }

    #[test]
    /// Tests workspace functions, ensuring we can get workspaces and new
    /// ones are properly generated with a root container when we request one
    /// that doesn't yet exist
    fn workspace_tests() {
        let mut tree = basic_tree();
        /* Simple workspace access tests */
        let workspace_1_ix = tree.tree.workspace_ix_by_name("1")
            .expect("Workspace 1 did not exist");
        assert_eq!(tree.tree[workspace_1_ix].get_type(), ContainerType::Workspace);
        assert_eq!(tree.tree[workspace_1_ix].get_name().unwrap(), "1");
        let workspace_2_ix = tree.tree.workspace_ix_by_name("2")
            .expect("Workspace 2 did not exist");
        assert_eq!(tree.tree[workspace_2_ix].get_type(), ContainerType::Workspace);
        assert_eq!(tree.tree[workspace_2_ix].get_name().unwrap(), "2");
        assert!(tree.tree.workspace_ix_by_name("3").is_none(),
                "Workspace three existed, expected it not to");
        /* init workspace tests */
        let output_ix = tree.active_ix_of(ContainerType::Output)
            .expect("No active output");
        let active_3_ix = tree.init_workspace("3".to_string(), output_ix);
        let workspace_3_ix = tree.tree.parent_of(active_3_ix).unwrap();
        assert!(tree.tree.workspace_ix_by_name("3").is_some(),
                "Workspace three still does not exist, even though we just initialized it");
        assert_eq!(tree.tree[workspace_3_ix].get_type(), ContainerType::Workspace);
        assert_eq!(tree.tree[workspace_3_ix].get_name().unwrap(), "3");
    }

    #[test]
    /// Tests the view functions
    fn view_tests() {
        let mut tree = basic_tree();
        let active_container = tree.active_container.expect("No active container");
        let parent_container = tree.tree.parent_of(active_container).unwrap();
        // When the active container is a view, add it as a sibling
        assert_eq!(tree.tree.children_of(parent_container).len(), 1);
        let old_active_view = tree.active_ix_of(ContainerType::View)
            .expect("Active container was not a view");
        let uuid_of_added: Uuid = tree.add_view(WlcView::root()).unwrap();
        assert_eq!(tree.tree.children_of(parent_container).len(), 2);
        assert!(! (old_active_view == tree.active_ix_of(ContainerType::View).unwrap()));
        let added_container_ix = tree.tree.lookup_id(uuid_of_added)
            .expect("Id of just added container doesn't match with a container!");
        tree.remove_container(added_container_ix)
            .expect("Could not remove container we just added!");
        // Can't remove a node twice without it giving us an error
        assert!(tree.remove_container(added_container_ix).is_err());
        assert_eq!(tree.active_ix_of(ContainerType::View).unwrap(), old_active_view);
        assert_eq!(tree.tree.children_of(parent_container).len(), 1);
        for _ in 1..2 {
            tree.remove_view(&WlcView::root()).expect("Could not remove view");
        }
    }

    #[test]
    fn remove_active_test() {
        let mut tree = basic_tree();
        let root_container = tree.tree[tree.tree.parent_of(tree.active_container.unwrap()).unwrap()].clone();
        tree.remove_active().unwrap();
        assert_eq!(tree.tree[tree.active_container.unwrap()], root_container);
    }

    #[test]
    fn add_output_test() {
        let mut tree = basic_tree();
        let new_output = WlcView::root().as_output();
        tree.add_output(new_output);
        let output_ix = tree.active_ix_of(ContainerType::Output).unwrap();
        let handle = match tree.tree[output_ix].get_handle().unwrap() {
            Handle::Output(output) => output,
            _ => panic!()
        };
        assert_eq!(handle, new_output);
        let workspace_ix = tree.tree.descendant_of_type(output_ix, ContainerType::Workspace).unwrap();
        assert_eq!(tree.tree[workspace_ix].get_name().unwrap(), "1");
        let active_ix = tree.active_container.unwrap();
        assert_eq!(tree.tree.parent_of(active_ix).unwrap(), workspace_ix);
        assert_eq!(tree.tree.children_of(active_ix).len(), 0);
    }

    #[test]
    /// Tests that we can remove the active container and have it properly reset
    fn basic_removal() {
        let mut tree = basic_tree();
        let active_view_ix = tree.active_container
            .expect("No active container on basic tree");
        assert!(tree.tree[active_view_ix].get_type() == ContainerType::View,
                "Active container was not a view");
        let workspace_of_active = tree.tree.ancestor_of_type(active_view_ix,
                                                             ContainerType::Workspace)
            .expect("View not part of workspace");
        // The next active container should be the root container of this workspace
        let new_active_container_ix = &tree.tree.children_of(workspace_of_active)[0];

        tree.remove_view_or_container(active_view_ix).unwrap();
        let new_active_container = tree.active_container
            .expect("Remove view invalidated the active container");
        assert_eq!(new_active_container, *new_active_container_ix);

    }

    #[test]
    fn toggle_layout_test() {
        {
            let mut tree = basic_tree();
            let root_container = tree.tree.parent_of(tree.active_container.unwrap()).unwrap();
            tree.active_container = Some(root_container);
            assert!(tree.tree.is_root_container(root_container));
            let layout = match tree.tree[root_container] {
                Container::Container { ref layout, .. } => layout.clone(),
                _ => panic!()
            };
            // default layout
            assert_eq!(layout, Layout::Horizontal);
            for new_layout in &[Layout::Vertical, Layout::Horizontal] {
                tree.toggle_active_layout(*new_layout).unwrap();
                let layout = match tree.tree[root_container] {
                    Container::Container { ref layout, .. } => layout.clone(),
                    _ => panic!()
                };
                assert_eq!(layout, *new_layout);
            }
        }
        /* Now test wrapping the active container in a new container */
        {
            let mut tree = basic_tree();
            let active_ix = tree.active_container.unwrap();
            let active_container = tree.tree[active_ix].clone();
            let old_parent = tree.tree[tree.tree.parent_of(active_ix).unwrap()]
                .clone();
            let old_layout = match old_parent {
                Container::Container { ref layout, ..} => layout.clone(),
                _ => panic!()
            };
            assert_eq!(old_layout, Layout::Horizontal);
            tree.toggle_active_layout(Layout::Vertical).unwrap();
            // should still be focused on the previous container.
            // though the active index might be different
            let active_ix = tree.active_container.unwrap();
            assert_eq!(active_container, tree.tree[active_ix]);
            let new_parent = tree.tree[tree.tree.parent_of(active_ix).unwrap()]
                .clone();
            let new_layout = match new_parent {
                Container::Container { ref layout, ..} => layout.clone(),
                _ => panic!()
            };
            assert!(old_parent != new_parent);
            assert_eq!(new_layout, Layout::Vertical);
        }
    }

    #[test]
    fn add_container_test() {
        let mut tree = basic_tree();
        let active_ix = tree.active_container.unwrap();
        let parent_ix = tree.tree.parent_of(active_ix).unwrap();
        let old_edge_weight = *tree.tree.get_edge_weight_between(parent_ix, active_ix)
            .unwrap();
        // First and only child, so the edge weight is 1
        assert_eq!(*old_edge_weight, 1);
        let geometry = Geometry {
            origin: Point { x: 0, y: 0},
            size: Size { w: 0, h: 0}
        };
        let new_container = Container::new_container(geometry);
        tree.add_container(new_container, active_ix).unwrap();
        let new_active_ix = tree.active_container.unwrap();
        // The view moved, since it was placed in the new container
        assert!(active_ix != new_active_ix);
        let new_container_ix = tree.tree.parent_of(new_active_ix).unwrap();
        let parent_ix = tree.tree.parent_of(new_container_ix).unwrap();
        let new_edge_weight = *tree.tree.get_edge_weight_between(parent_ix, new_container_ix)
            .unwrap();
        assert_eq!(new_edge_weight, old_edge_weight);

    }

    #[test]
    fn non_root_container_auto_removal_test() {
        let mut tree = basic_tree();
        tree.switch_to_workspace("2");
        /* Remove first View */
        let root_container = tree.tree.children_of(tree.active_ix_of(ContainerType::Workspace)
                                                   .expect("No active workspace"))[0];
        let num_children = tree.tree.children_of(root_container).len();
        assert_eq!(num_children, 1);
        let active_view_ix = tree.active_container.unwrap();
        assert_eq!(tree.tree[active_view_ix].get_type(), ContainerType::View);
        tree.remove_view_or_container(active_view_ix).unwrap();
        /* Remove the other view*/
        let active_view_ix = tree.active_container.unwrap();
        assert_eq!(tree.tree[active_view_ix].get_type(), ContainerType::View);
        tree.remove_view_or_container(active_view_ix).unwrap();
        /* This should remove the other container,
        the count of the root container should be 0 */
        let active_ix = tree.active_container.unwrap();
        assert!(tree.tree.is_root_container(active_ix));
        let root_container = tree.tree.children_of(tree.active_ix_of(ContainerType::Workspace)
                                                   .expect("No active workspace"))[0];
        let num_children = tree.tree.children_of(root_container).len();
        assert_eq!(num_children, 0);
        assert!(tree.remove_view_or_container(active_view_ix).is_err());
    }

    #[test]
    fn move_to_workspace_test() {
        // NOTE Need to test adding to workspace with stuff already in that workspace
        let mut tree = basic_tree();
        /* Make sure sending to the current workspace does nothing */
        let old_view = tree.tree[tree.active_container.unwrap()].clone();
        tree.send_active_to_workspace("1");
        assert_eq!(old_view, tree.tree[tree.active_container.unwrap()]);
        //let old_view = tree.tree[tree.active_container.unwrap()].clone();
        tree.send_active_to_workspace("3");
        // Trying to send the root container does nothing
        tree.send_active_to_workspace("3");
        let active_ix = tree.active_container.unwrap();
        assert!(tree.tree.is_root_container(active_ix));
        tree.switch_to_workspace("3");
        let active_ix = tree.active_container.unwrap();
        // Switch to new workspace, should be focused on the old view
        assert_eq!(old_view, tree.tree[active_ix]);
    }

    #[test]
    fn auto_workspace_adding() {
        let mut tree = basic_tree();
        let output = tree.active_ix_of(ContainerType::Output).unwrap();
        // there are two workspaces at the beginning, 1 and 2
        assert_eq!(tree.tree.children_of(output).len(), 2);
        tree.switch_to_workspace("1");
        // Switching to current doesn't change that
        assert_eq!(tree.tree.children_of(output).len(), 2);
        // Switching to other doesn't either
        tree.switch_to_workspace("2");
        assert_eq!(tree.tree.children_of(output).len(), 2);
        // This does add the new one
        tree.switch_to_workspace("3");
        assert_eq!(tree.tree.children_of(output).len(), 3);
    }


    #[test]
    /// Ensures that toggle horizontal key (<Leader> + e) does the same thing as it does in i3.
    /// To reiterate: it should always make the active view's parent container( or the container
    /// itself if the active container is a container, not a view) have the horizontal layout
    /// _unless_ it's already horizontal, in which case the layout should be vertical
    fn tiling_toggle_key() {
        let mut tree = basic_tree();
        // active container is the first view, so it should just change it's root.
        let parent = tree.tree.parent_of(tree.active_container.unwrap()).unwrap();
        match tree.tree[parent] {
            Container::Container { ref layout, .. } => {
                // default is horizontal
                assert_eq!(*layout, Layout::Horizontal)
            },
            _ => unreachable!()
        }
        tree.toggle_cardinal_tiling();
        let parent = tree.tree.parent_of(tree.active_container.unwrap()).unwrap();
        match tree.tree[parent] {
            Container::Container { ref layout, .. } => {
                // default is horizontal
                assert_eq!(*layout, Layout::Vertical)
            },
            _ => unreachable!()
        }
        // and back again
        tree.toggle_cardinal_tiling();
        let parent = tree.tree.parent_of(tree.active_container.unwrap()).unwrap();
        match tree.tree[parent] {
            Container::Container { ref layout, .. } => {
                // default is horizontal
                assert_eq!(*layout, Layout::Horizontal)
            },
            _ => unreachable!()
        }
    }

    #[test]
    fn move_focus_simple_test() {
        let mut tree = basic_tree();
        let directions = [Direction::Up, Direction::Right,
                          Direction::Down, Direction::Left];
        let old_active_ix = tree.active_container.clone();
        tree.active_container = None;
        for direction in &directions {
            // should do nothing when there is no active container
            tree.move_focus(*direction).unwrap();
            assert_eq!(tree.active_container, None);
        }
        tree.active_container = old_active_ix;
        for direction in &directions {
            // should do nothing when there are no other views to move to
            tree.move_focus(*direction).unwrap();
            assert_eq!(tree.active_container, old_active_ix);
        }
        // set to root container
        let root_container_ix = tree.tree.parent_of(old_active_ix.unwrap()).unwrap();
        assert!(tree.tree.is_root_container(root_container_ix));
        tree.active_container = Some(root_container_ix);
        for direction in &directions {
            tree.move_focus(*direction).unwrap();
            assert_eq!(tree.active_container, Some(root_container_ix));
        }
    }

    #[test]
    fn move_focus_complex_test() {
        let mut tree = basic_tree();
        tree.switch_to_workspace("2");
        // We are focused on the far left container
        let left_ix = tree.active_container.unwrap();
        // Get the next one
        tree.move_focus(Direction::Right).unwrap();
        // Make sure we moved
        let right_ix = tree.active_container.unwrap();
        assert!(left_ix != right_ix);
        // make a vertical container here, try to move back to the original
        let new_container = tree.tree[right_ix].clone();
        tree.toggle_active_layout(Layout::Vertical).unwrap();
        assert_eq!(new_container, tree.tree[tree.active_container.unwrap()]);
        // Add a new view, it'll be below us
        tree.add_view(WlcView::root()).unwrap();
        // Move up, should be on the original view still
        tree.move_focus(Direction::Up).unwrap();
        assert_eq!(new_container, tree.tree[tree.active_container.unwrap()]);
        // Move Right, should be right where we are, nothing to the right
        tree.move_focus(Direction::Right).unwrap();
        assert_eq!(new_container, tree.tree[tree.active_container.unwrap()]);
        // Move left, be back on the very first one
        tree.move_focus(Direction::Left).unwrap();
        assert_eq!(left_ix, tree.active_container.unwrap());
        // Move left, be back on the very first one
    }

    #[test]
    fn switch_to_workspace_test() {
        let mut tree = basic_tree();
        let old_active = tree.active_container.clone();
        let current_workspace_ix = tree.active_ix_of(ContainerType::Workspace).unwrap();
        tree.active_container = None;
        tree.switch_to_workspace("3");
        // didn't move, because we have no active container
        tree.active_container = old_active;
        assert_eq!(tree.active_ix_of(ContainerType::Workspace).unwrap(), current_workspace_ix);
        tree.switch_to_workspace("1");
        // didn't move, because we aren't going anywhere different (same workspace)
        assert_eq!(tree.active_ix_of(ContainerType::Workspace).unwrap(), current_workspace_ix);
        tree.active_container = tree.active_ix_of(ContainerType::Output);
        tree.switch_to_workspace("3");
        // didn't move, because we aren't focused on something with a workspace
        tree.active_container = old_active;
        assert_eq!(tree.active_ix_of(ContainerType::Workspace).unwrap(), current_workspace_ix);
    }

    #[test]
    fn active_is_root_test() {
        let mut tree = basic_tree();
        assert_eq!(tree.active_is_root(), false);
        tree.remove_active().unwrap();
        assert_eq!(tree.active_is_root(), true);
        tree.active_container = None;
        assert_eq!(tree.active_is_root(), false);
        assert!(tree.remove_active().is_err());
    }

    #[test]
    fn can_move_workspaces_with_no_active() {
        let mut tree = basic_tree();
        let workspace_ix = tree.active_ix_of(ContainerType::Workspace)
            .expect("Active container wasn't set properly in basic_tree!");
        assert_eq!(tree.tree[workspace_ix].get_name(), Some("1"));
        tree.active_container = None;
        tree.switch_to_workspace("2");
        let workspace_ix = tree.active_ix_of(ContainerType::Workspace)
            .expect("The new workspace was not set!");
        assert_eq!(tree.tree[workspace_ix].get_name(), Some("2"));
    }

    #[test]
    fn setting_container_active_sets_workspace_active() {
        let mut tree = basic_tree();
        let workspace_ix = tree.active_ix_of(ContainerType::Workspace)
            .expect("Active container wasn't set properly in basic_tree!");
        assert_eq!(tree.tree[workspace_ix].get_name(), Some("1"));
        tree.switch_to_workspace("2");
        let workspace_ix = tree.active_ix_of(ContainerType::Workspace)
            .expect("Active container wasn't set properly in basic_tree!");
        assert_eq!(tree.tree[workspace_ix].get_name(), Some("2"));
    }
}
