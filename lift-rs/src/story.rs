// Lift Interpreter Core
use std::fmt;
use std::collections::HashMap;
use std::rc::Rc;
use regex::Regex;
use lazy_static::lazy_static;
use serde::{Serialize, Deserialize};
use crate::content::{Page, Content, Action, PageAction};
use crate::parser::ContentError;
use crate::expression::StateManager;
use crate::value::Value;

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum Element {
    Text(String),
    Link(String, String),
    ContentLink(String, PageAction),
    JumpLink(String, String, PageAction),
    Input(String, PageAction),
    Error(String)
}

enum StoryAction {
    Goto(String),
    Halt
}

struct StoryResult {
    output: Vec<Element>,
    action: StoryAction
}

impl StoryResult {
    fn new() -> Self {
        StoryResult { output: Vec::<Element>::new(), action: StoryAction::Halt }
    }

    fn combine(&mut self, mut result: StoryResult) {
        self.output.append(&mut result.output);
        self.action = result.action;
    }

    fn push(&mut self, element: Element) {
        self.output.push(element);
    }
}

pub struct Story {
    first_page: String,
    pages: HashMap<String, Page>
}

pub enum StoryError {
    Content(ContentError, String, usize),
    DuplicatePage(String, usize)
}

impl fmt::Display for StoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoryError::Content(error, page, line) =>
                write!(f, "Parsing error on page '{}', line {}:\n{}", page, line, error),
            StoryError::DuplicatePage(page, line) =>
                write!(f, "Duplicate page '{}' on line {}", page, line)
        }
    }
}

impl Story {
    pub fn new(source: &str) -> Result<Self, StoryError> {
        lazy_static! {
            static ref HEADER_REGEX: Regex = Regex::new(r"^#+(?P<title>.+)").unwrap();
        }

        let mut pages = HashMap::<String, Page>::new();
        let mut content_acumulator = "".to_string();
        let mut first_page: Option<&str> = None;
        let mut current_page: Option<&str> = None;

        let mut page_line: usize = 1;
        for (line_number, line) in source.lines().enumerate() {
            if let Some(capture) = HEADER_REGEX.captures(line) {
                if let Some(title) = current_page {
                    let page = Self::parse_page(page_line, title, &content_acumulator)?;
                    pages.insert(title.to_string(), page);
                    content_acumulator = "".to_string();
                }
                page_line = line_number + 1;
                let title = capture.name("title").unwrap().as_str().trim();
                if pages.contains_key(title) {
                    return Err(StoryError::DuplicatePage(title.to_string(), page_line));
                }
                if first_page == None {
                    first_page = current_page
                }
                current_page = Some(title);
            } else if let Some(_) = current_page {
                content_acumulator += &format!("{}\n", line);
            }
        }
        if let Some(title) = current_page {
            let page = Self::parse_page(page_line, title, &content_acumulator)?;
            pages.insert(title.to_string(), page);
            if first_page == None {
                first_page = Some(title)
            }
        }
        Ok(Story {pages, first_page: first_page.unwrap_or("").to_string()})
    }

    fn parse_page(line_number: usize, title: &str, content: &str) -> Result<Page, StoryError> {
        Page::parse(title, content).map_err(|(size, error)| StoryError::Content (
            error,
            title.to_string(),
            line_number + content[..size].lines().count()
        ))
    }

    fn get_action(&self, action: PageAction) -> Option<&Vec<Content>> {
        let page = self.pages.get(&action.page)?;
        return page.actions.get(action.index);
    }
}

#[derive(Serialize, Deserialize)]
struct State {
    current_page: String,
    global: HashMap<String, Value>,
    local: HashMap<String, HashMap<String, Value>>,
    output: Vec<Element>
}

impl State {
    fn new(first_page: &str) -> State {
        State {
            current_page: first_page.to_string(),
            global: HashMap::new(),
            local: HashMap::new(),
            output: vec![]
        }
    }

    fn get(&self, page: &str, variable: &str) -> Option<&Value> {
        if let Some(state) = self.local.get(page) {
            if let Some(value) = state.get(variable) {
                return Some(value);
            }
        }
        return self.global.get(variable);
    }

    fn set(&mut self, variable: &str, value: Value) {
        self.global.insert(variable.to_string(), value);
    }

    fn set_index(&mut self, variable: &str, indices: &Vec<Value>, value: Value) {
        if indices.is_empty() {
            return self.set(variable, value);
        }
        if let Some(var) = self.global.get_mut(variable) {
            if let Some(reference) = var.get_mut(indices) {
                *reference = value;
            }
        }
    }

    fn set_local(&mut self, variable: &str, value: Value) -> Option<()> {
        let state = match self.local.get_mut(&self.current_page) {
            Some(state) => state,
            None => {
                self.local.insert(self.current_page.to_string(), HashMap::new());
                self.local.get_mut(&self.current_page)?
            }
        };
        state.insert(variable.to_string(), value);
        Some(())
    }

    fn set_local_index(&mut self, variable: &str, indices: &Vec<Value>, value: Value) -> Option<()> {
        if indices.is_empty() {
            return self.set_local(variable, value);
        }
        let state = self.local.get_mut(&self.current_page)?;
        let var = state.get_mut(variable)?;
        let reference = var.get_mut(indices)?;
        Some(*reference = value)
    }
}

impl StateManager for State {
    fn get(&self, variable: &str) -> Option<&Value> {
        self.get(&self.current_page, variable)
    }
}

pub struct Interpreter {
    story: Rc<Story>,
    state: State
}

impl Interpreter {
    pub fn new(story: Story) ->  Self {
        let state = State::new(&story.first_page);
        Interpreter {
            story: Rc::new(story),
            state,
        }
    }

    fn process_result(&mut self, result: StoryResult, index: usize) {
        match result.action {
            StoryAction::Halt => {
                self.state.output.splice(index..index+1, result.output);
            }
            StoryAction::Goto(page) => {
                self.state.current_page = page;
                self.play();
                self.state.output.splice(0..0, result.output);
            }
        }
    }

    pub fn send(&mut self, index: usize, value: Value) {
        let element: Option<Element> = self.state.output.get(index).cloned();
        let story = &Rc::clone(&self.story);
        if let Some(Element::Link(_, destination)) = element {
            self.state.current_page = destination.to_string();
            self.play();
        }
        else if let Some(Element::ContentLink(_, action)) = element {
            if let Some(content) = story.get_action(action) {
                let result = self.eval(content);
                self.process_result(result, index);
            }
        }
        else if let Some(Element::JumpLink(_, destination, action)) = element {
            if let Some(content) = story.get_action(action) {
                let mut result = self.eval(content);
                result.action = StoryAction::Goto(destination.to_string());
                self.process_result(result, index);
            }
        }
        else if let Some(Element::Input(variable, action)) = element {
            if let Some(content) = story.get_action(action) {
                self.state.set_local(&variable, value);
                let result = self.eval(content);
                self.process_result(result, index);
            }
        }
    }

    pub fn play(&mut self) {
        self.state.output.clear();
        let story: &Story = &Rc::clone(&self.story);
        loop {
            if let Some(page) = story.pages.get(&self.state.current_page) {
                let mut result = self.eval(&page.content);
                self.state.output.append(&mut result.output);
                match result.action {
                    StoryAction::Halt => break,
                    StoryAction::Goto(p) => {
                        self.state.output.clear();
                        self.state.current_page = p
                    }
                }
            }
            else {
                self.state.output.push(Element::Error(format!("Invalid page: '{}'", self.state.current_page)));
                break;
            }
        }
    }

    pub fn output(&self) -> &Vec<Element> {
        &self.state.output
    }

    pub fn dump_state(&self) -> Option<String> {
        if let Ok(json) = serde_json::to_string(&self.state) {
            return Some(json)
        }
        None
    }

	pub fn load_state(&mut self, json: &str) -> serde_json::Result<()> {
		self.state = serde_json::from_str(json)?;
		Ok(())
	}

    fn eval(&mut self, content: &Vec<Content>) -> StoryResult {
        let mut result = StoryResult::new();
        let mut if_action: Option<bool> = None;
        let story: &Story = &Rc::clone(&self.story);
        for element in content.iter() {
            match element {
                Content::Text(s) => result.push(Element::Text(s.eval(&self.state))),
                Content::Link(link) => {
                    let element = match link {
                        Action::Normal{title, destination} => {
                            Element::Link(title.eval(&self.state), destination.eval(&self.state))
                        }
                        Action::Content{title, action} => {
                            Element::ContentLink(title.eval(&self.state), action.clone())
                        }
                        Action::JumpLink{title, destination, action} => {
                            Element::JumpLink(title.eval(&self.state), destination.eval(&self.state), action.clone())
                        }
                        Action::Input{variable, action} => {
                            Element::Input(variable.to_string(), action.clone())
                        }
                    };
                    result.push(element);
                }
                Content::Goto(page) => {result.action = StoryAction::Goto(page.eval(&self.state))},
                Content::Import(page_title) => {
                    if let Some(page) = story.pages.get(&page_title.eval(&self.state)) {
                        let import_result = self.eval(&page.content);
                        result.combine(import_result);
                    }
                }
                Content::Set{local, variable, indices, expression} => {
                    let value = expression.eval(&self.state);
                    let ind: Vec<_> = indices.iter().map(|x| x.eval(&self.state)).collect();
                    if *local {
                        self.state.set_local_index(variable, &ind, value);
                    }
                    else {
                        self.state.set_index(variable, &ind, value);
                    }
                }
                Content::If{expression, content} => {
                    if_action = Some(expression.eval(&self.state).is_true());
                    if let Some(true) = if_action {
                        let content_result = self.eval(content);
                        result.combine(content_result);
                    }
                }
                Content::ElseIf{expression, content} => {
                    if let Some(false) = if_action {
                        if_action = Some(expression.eval(&self.state).is_true());
                        if let Some(true) = if_action {
                            let content_result = self.eval(content);
                            result.combine(content_result);
                        }
                    }
                }
                Content::Else { content } => {
                    if let Some(false) = if_action {
                        if_action = None;
                        let content_result = self.eval(content);
                        result.combine(content_result);
                    }
                }
                Content::For { index, variable, expression, content} => {
                    let iterator_value = expression.eval(&self.state);
                    for (i, value) in iterator_value.iter() {
                        if let Some(index) = index {
                            self.state.set_local(index, i);
                        }
                        self.state.set_local(variable, value);
                        let content_result = self.eval(content);
                        result.combine(content_result);
                        if let StoryAction::Goto(_) = result.action {
                            break;
                        }
                    }
                }
                Content::While {expression, content} => {
                    while expression.eval(&self.state).is_true() {
                        let content_result = self.eval(content);
                        result.combine(content_result);
                        if let StoryAction::Goto(_) = result.action {
                            break;
                        }
                    }
                }
                Content::Error(e) => result.push(Element::Error(e.to_string()))
            }
            if let StoryAction::Goto(_) = result.action {
                return result;
            }
        }
        return result;
    }
}
