//! A PDF's outline (its table of contents) and its links, and where each
//! leads. poppler-rs leaves out poppler's actions, so they are read here
//! through its C interface.

use std::ffi::CStr;

use gtk::glib::{self, translate::ToGlibPtr};
use poppler::ffi;

/// Where an outline entry or a link leads.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// A page (from 0) and, when given, how far down it, in points from
    /// its top.
    Page(usize, Option<f64>),
    Uri(String),
}

/// An outline entry: its depth (from 0), title and target.
pub struct Entry {
    pub depth: usize,
    pub title: String,
    pub target: Option<Target>,
}

/// A link on a page: its area, in points from the page's top-left corner.
pub struct Link {
    pub area: (f64, f64, f64, f64),
    pub target: Target,
}

/// The document's outline, depth first; empty if it has none.
pub fn outline(document: &poppler::Document) -> Vec<Entry> {
    let mut entries = Vec::new();
    let doc: *mut ffi::PopplerDocument = document.to_glib_none().0;
    unsafe {
        let iter = ffi::poppler_index_iter_new(doc);
        if !iter.is_null() {
            walk(document, iter, 0, &mut entries);
            ffi::poppler_index_iter_free(iter);
        }
    }
    entries
}

unsafe fn walk(
    document: &poppler::Document,
    iter: *mut ffi::PopplerIndexIter,
    depth: usize,
    entries: &mut Vec<Entry>,
) {
    // Deep outlines are rare; past this, the rest is left out.
    if depth > 16 {
        return;
    }
    loop {
        unsafe {
            let action = ffi::poppler_index_iter_get_action(iter);
            if !action.is_null() {
                let title = cstr((*action).any.title).unwrap_or_default();
                entries.push(Entry {
                    depth,
                    title: title.trim().to_owned(),
                    target: target(document, action),
                });
                ffi::poppler_action_free(action);
            }
            let child = ffi::poppler_index_iter_get_child(iter);
            if !child.is_null() {
                walk(document, child, depth + 1, entries);
                ffi::poppler_index_iter_free(child);
            }
            if ffi::poppler_index_iter_next(iter) == glib::ffi::GFALSE {
                break;
            }
        }
    }
}

/// The links on `page`, `height` points tall.
pub fn links(document: &poppler::Document, page: &poppler::Page, height: f64) -> Vec<Link> {
    let mut links = Vec::new();
    let page_ptr: *mut ffi::PopplerPage = page.to_glib_none().0;
    unsafe {
        let list = ffi::poppler_page_get_link_mapping(page_ptr);
        let mut node = list;
        while !node.is_null() {
            let mapping = (*node).data as *mut ffi::PopplerLinkMapping;
            if !mapping.is_null()
                && let Some(target) = target(document, (*mapping).action)
            {
                let r = (*mapping).area;
                // Link areas count from the page's bottom; the screen from its top.
                links.push(Link {
                    area: (r.x1, height - r.y2, r.x2, height - r.y1),
                    target,
                });
            }
            node = (*node).next;
        }
        ffi::poppler_page_free_link_mapping(list);
    }
    links
}

unsafe fn target(document: &poppler::Document, action: *mut ffi::PopplerAction) -> Option<Target> {
    if action.is_null() {
        return None;
    }
    unsafe {
        match (*action).type_ {
            ffi::POPPLER_ACTION_URI => cstr((*action).uri.uri).map(Target::Uri),
            ffi::POPPLER_ACTION_GOTO_DEST => destination(document, (*action).goto_dest.dest),
            _ => None,
        }
    }
}

unsafe fn destination(document: &poppler::Document, dest: *mut ffi::PopplerDest) -> Option<Target> {
    if dest.is_null() {
        return None;
    }
    unsafe {
        if (*dest).type_ == ffi::POPPLER_DEST_NAMED {
            // A name to look up in the document.
            let name = (*dest).named_dest;
            if name.is_null() {
                return None;
            }
            let doc: *mut ffi::PopplerDocument = document.to_glib_none().0;
            let found = ffi::poppler_document_find_dest(doc, name);
            if found.is_null() {
                return None;
            }
            let target = page_target(document, &*found);
            ffi::poppler_dest_free(found);
            return target;
        }
        page_target(document, &*dest)
    }
}

fn page_target(document: &poppler::Document, dest: &ffi::PopplerDest) -> Option<Target> {
    // Pages count from 1 here.
    let index = usize::try_from(dest.page_num).ok()?.checked_sub(1)?;
    let height = document.page(index as i32)?.size().1;
    // `top` counts from the page's bottom; 0 when the link gives none.
    let top = (dest.top > 0.0 && dest.top <= height).then_some(height - dest.top);
    Some(Target::Page(index, top))
}

unsafe fn cstr(ptr: *const std::ffi::c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    Some(unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned())
}
